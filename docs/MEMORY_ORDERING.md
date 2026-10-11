# Memory Ordering and Cache Analysis of the SPSC Ring

Everything below was measured on 2026-10-10 on the development laptop:

| | |
| --- | --- |
| CPU | AMD Ryzen 7 7735HS (8 cores / 16 threads, Zen 3+), Windows 11 power plan "Turbo", threads not pinned |
| Rust | rustc 1.93.1 (LLVM 21.1.8), `--release`, default `target-cpu` (generic x86-64) |
| C++ | g++ 14.2.0 (MSYS2 UCRT64), `-std=c++20 -O3`, generic x86-64 |
| Harness | `examples/ring_throughput.rs` and `cpp/bench_ring.cpp`, same methodology, 30 iterations, medians |

Absolute numbers on this machine move by 1.3–1.7x between sessions (see
[METRICS.md](METRICS.md#re-measurement-2026-10-10)). Every comparison below was
taken within one session, back to back, so the *relative* numbers are the
result; the absolute ones are context.

## 1. What the compiler emits: acquire/release vs SeqCst

The README claims the ring's `Acquire`/`Release` cursor accesses compile to plain
`mov` on x86-64. `scripts/asm_ordering.sh` checks that claim for both languages
by compiling out-of-line `push`/`pop` probes and extracting the function bodies
into [`docs/asm/`](asm/):

| build | head store (producer) | tail store (consumer) | `xchg` / `mfence` / `lock` in push+pop |
| --- | --- | --- | --- |
| Rust, acquire/release (shipped) | `movq %r8, 64(%r10)` | `movq %r8, 128(%r9)` | 0 / 0 / 0 |
| Rust, `--cfg ring_seqcst` | `xchgq %rax, 64(%r10)` | `xchgq %rdx, 128(%r9)` | 2 / 0 / 0 |
| C++, acquire/release (shipped) | `movq %rax, 64(%r8)` | `movq %rax, 128(%rdx)` | 0 / 0 / 0 |
| C++, `-DFERROX_SEQ_CST` | `xchgq 64(%r8), %rdx` | `xchgq 128(%rdx), %r10` | 2 / 0 / 0 |

Reading: on x86-64 (TSO) a release store needs no fence, so both LLVM and GCC
emit an ordinary `mov`; the acquire loads are ordinary `mov`s too. A SeqCst
store must be globally ordered against later loads, which x86 only guarantees
for locked instructions, so both compilers turn the store into `xchg` (an
implicitly `lock`ed read-modify-write that also drains the store buffer). The
`64(...)` / `128(...)` offsets are the two cache-line-padded cursors; the two
languages lay the struct out identically.

The claim holds. The refactor that made the ring loom-checkable (a `Slot<T>`
wrapper around the `UnsafeCell`) produces an instruction stream for push+pop
that is identical to the pre-refactor code (64 instructions, compared after
normalizing register names and labels).

## 2. What the orderings cost at runtime

Same binary layout, one knob flipped at a time. Median of 30 iterations of
1M items through an 8192-slot ring; `push_pop` is single-threaded push-then-pop.

| variant | Rust u64 (ms) | Rust Order48 (ms) | Rust push+pop (ns) | C++ u64 (ms) | C++ Order48 (ms) | C++ push+pop (ns) |
| --- | --- | --- | --- | --- | --- | --- |
| shipped: acq/rel, padded, cursor cache | 4.419 | 13.206 | 4.044 | 5.174 | 16.387 | 1.138 |
| `ring_seqcst` / `FERROX_SEQ_CST` | 23.647 | 30.507 | 5.374 | 30.959 | 34.188 | 4.147 |
| `ring_no_padding` / `FERROX_NO_PADDING` | 9.091 | 15.597 | 3.424 | 9.092 | 15.580 | 1.385 |
| `ring_no_cursor_cache` / `FERROX_NO_CURSOR_CACHE` | 7.508 | 13.014 | 3.554 | 7.051 | 16.850 | 1.301 |

Observations, stated as measured:

- **SeqCst is the expensive one.** 5.4x slower (Rust) / 6.0x slower (C++) on
  the u64 workload, 2.1–2.3x on the 48-byte workload. Each `xchg` is a full
  barrier that waits for the store buffer to drain, and with two threads
  hammering two cache lines that wait is a cross-core round trip.
- **False sharing costs 2x on small payloads.** Putting `head` and `tail` on one
  cache line doubles u64 transfer time in both languages (4.4 → 9.1 ms, 5.2 →
  9.1 ms): every producer store invalidates the line the consumer is about to
  read and vice versa. For 48-byte payloads the effect shrinks to 1.2x (Rust)
  and vanishes in C++ (within noise), because copying the payload and touching
  its slot's cache line dominates.
- **Cursor caching matters for small payloads only.** Dropping the local copy
  of the remote cursor costs 1.7x (Rust) / 1.4x (C++) on u64 and nothing
  measurable on Order48. Without the cache every push does an acquire load of a
  line the other core keeps dirtying.
- **Single-thread push+pop barely moves** with padding and cursor caching
  (as expected: one core, no coherence traffic) and gets 1.3x (Rust) to 3.6x
  (C++) slower under SeqCst, which is the raw cost of two `xchg`.

## 3. Rust vs C++ on the same algorithm

Two independent runs per language, interleaved (Rust, C++, Rust, C++).
Median of 30 iterations; the minimum is also shown because this workload
includes thread creation and the OS scheduler adds a long tail.

| workload | Rust run 1 | Rust run 2 | C++ run 1 | C++ run 2 |
| --- | --- | --- | --- | --- |
| u64 / 1M, median ms | 4.419 | 4.751 | 5.174 | 5.417 |
| u64 / 1M, min ms | 2.716 | 2.630 | 2.425 | 3.769 |
| Order48 / 1M, median ms | 13.206 | 14.280 | 16.387 | 17.125 |
| Order48 / 1M, min ms | 5.985 | 6.712 | 5.539 | 5.467 |
| push+pop single-thread, median ns/op | 4.044 | 2.806 | 1.138 | 1.847 |

What this does and does not show:

- **Cross-thread throughput is a tie within noise.** Medians favor Rust by
  10–20%, minimums favor C++ by 5–10%, and the spread between two runs of the
  same binary is as large as the gap between languages. The transfer cost is
  cache-coherence traffic, which neither compiler can change.
- **Single-threaded push+pop was faster in C++, by 1.5–3.5x depending on the
  run** (C++ 1.1–1.8 ns vs Rust 2.3–4.0 ns across every run in this session).
  On 2026-10-10 this was reported as unexplained, with the `Result` return
  convention and LLVM-vs-GCC scheduling as candidates. Both guesses were
  wrong; section 5 has the actual cause (a register-promotion failure caused
  by how `Arc` is dropped) and the fix. It never showed up in the two-thread
  numbers because there the loop waits on cache lines, not on instructions.
- GCC was used for C++ and LLVM for Rust, so this is also a GCC-vs-LLVM
  comparison. `CXX=clang++ scripts/asm_ordering.sh` and
  `clang++ -O3 cpp/bench_ring.cpp` give the LLVM-vs-LLVM view on a machine
  that has clang.

## 4. What the bounds check costs (memory safety, quantified)

Safe Rust indexes the slot array with `buffer[cursor & mask]`. The index is
always in range (`mask == len - 1`, `len` a power of two) but the type system
cannot see that, so the compiler keeps a `cmp` + never-taken `jae` on every
push and pop (visible in [`docs/asm/rust_ring_acqrel.s`](asm/rust_ring_acqrel.s)
as the branch to `panic_bounds_check`). The `--cfg ring_unchecked_index` knob
replaces it with `get_unchecked` so the cost can be measured. Three alternating
runs of each binary:

| run | default push+pop ns | unchecked push+pop ns | default u64 ms | unchecked u64 ms | default Order48 ms | unchecked Order48 ms |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 2.653 | 2.296 | 5.043 | 7.399 | 13.974 | 14.215 |
| 2 | 3.420 | 2.614 | 5.264 | 5.691 | 15.061 | 14.826 |
| 3 | 2.840 | 2.478 | 4.554 | 6.270 | 12.570 | 16.128 |

The bounds check costs 10–25% in the single-threaded loop and nothing
measurable across threads (the unchecked build was slower there in all three
runs, which is within this harness's noise, not a real effect). Conclusion:
keep the check. It closes some but not all of the gap to C++ in section 3, and
it is free where the ring actually runs, between two cores.

## 5. The single-thread gap, explained and closed (2026-10-11)

`scripts/asm_loop.sh` compiles the exact `run_latency` loop of both harnesses
out of line (`latency_bench` in `scripts/asm_probe.rs` and `cpp/asm_probe.cpp`)
and writes the bodies to [`docs/asm/`](asm/). Reading them side by side:

| | g++ 14.2 `latency_bench` | rustc 1.93 `latency_bench`, before | rustc 1.93, after the fix |
| --- | --- | --- | --- |
| where the four cursor fields live during the loop | registers (`rax`, `r10`, `rdx`, `r11`) | the stack: every iteration loads and stores `cached_head`, `cached_tail`, and reloads the `Arc` pointer | registers (`r8+rbx`, `r10`, `r11`, `r9+rbx`) |
| loads + stores to the producer/consumer structs per iteration | 0 | 11 | 0 |
| loop body instructions | 25 | 38 | 38, of which 6 are the two bounds checks |

The before-loop has a loop-carried dependency through memory: the cursor is
stored to the stack at the end of one iteration and loaded back at the start
of the next, which costs a store-to-load forwarding latency (about 4–5 cycles
on Zen 3) on the critical path, where GCC's version has a 1-cycle register
dependency. That, not instruction count, is the 1.5–3.5x.

**Why LLVM kept the fields in memory.** The LLVM IR (`rustc --emit llvm-ir`)
shows the `Producer`/`Consumer` allocas passed by pointer to
`alloc::sync::Arc::drop_slow`, in the panic-cleanup block of the loop (any
`unwrap()` or bounds check can unwind, and unwinding must drop the two
locals). `Arc`'s `Drop` calls the out-of-line `drop_slow(&mut self)`, and
`&mut self.inner` is the address of the containing struct. Once an alloca's
address escapes into a call, SROA/mem2reg will not promote any of its fields.
Three experiments pin this down:

| build of the probe | cursors in registers? |
| --- | --- |
| default | no |
| `-C panic=abort` (no cleanup block; `drop_slow` now called on the normal exit path) | no |
| `-C panic=abort` + `mem::forget` the ring (no `drop_slow` call at all) | **yes** |

GCC's `shared_ptr` release passes the *control block* pointer (loaded from the
field) to its cold path, never the address of the local, so the C++ locals
were never pinned.

**Fix** (`src/ring.rs`, `Shared<T>`): the `Arc` is held in a `ManuallyDrop`
and dropped by value from an explicit `Drop` impl, so the only address that
reaches `drop_slow` is a temporary's. The push/pop functions are unchanged
(`docs/asm/rust_ring_acqrel.s` differs only in label numbers; 64
instructions, fence census unchanged) and loom still passes.

**Measured** (Windows, same session, 10 × 1M push+pop pairs per run, three
rounds, each round running the three binaries back to back):

| round | Rust before, median ns | Rust after, median ns | C++ (g++), median ns |
| --- | --- | --- | --- |
| 1 | 2.853 | 1.592 | 1.742 |
| 2 | 3.486 | 2.810 | 1.751 |
| 3 | 3.627 | 2.122 | 1.467 |

The pinned, 40-sample, statistically tested version of this comparison (Linux
and Windows) is in [METHODOLOGY.md](METHODOLOGY.md) and
[ROUND2_REPORT.md](../ROUND2_REPORT.md). Remaining difference, if any, is the
bounds check of section 4.

The general lesson is portable beyond this ring: in Rust, a hot loop over a
local that owns an `Arc` (or anything whose `Drop` takes `&mut self` into an
out-of-line function) can silently lose register promotion for *all* of that
local's fields. The symptom is a loop that stores a counter to the stack and
reloads it on the next iteration; the diagnosis is `--emit llvm-ir` and a
search for the alloca being passed to a call.

## Reproduce

```bash
# assembly evidence (writes docs/asm/*.s and prints the fence census)
bash scripts/asm_ordering.sh

# Rust harness: default, then each knob
cargo run --release --example ring_throughput -- 30
RUSTFLAGS="--cfg ring_seqcst"          cargo run --release --example ring_throughput -- 30
RUSTFLAGS="--cfg ring_no_padding"      cargo run --release --example ring_throughput -- 30
RUSTFLAGS="--cfg ring_no_cursor_cache" cargo run --release --example ring_throughput -- 30
RUSTFLAGS="--cfg ring_unchecked_index" cargo run --release --example ring_throughput -- 30

# C++ harness: default, then each knob
g++ -std=c++20 -O3 -pthread cpp/bench_ring.cpp -o cpp/bench_ring && cpp/bench_ring 30
g++ -std=c++20 -O3 -pthread -DFERROX_SEQ_CST         cpp/bench_ring.cpp -o cpp/bench_ring && cpp/bench_ring 30
g++ -std=c++20 -O3 -pthread -DFERROX_NO_PADDING      cpp/bench_ring.cpp -o cpp/bench_ring && cpp/bench_ring 30
g++ -std=c++20 -O3 -pthread -DFERROX_NO_CURSOR_CACHE cpp/bench_ring.cpp -o cpp/bench_ring && cpp/bench_ring 30

# C++ correctness tests
g++ -std=c++20 -O2 -pthread cpp/test_ring.cpp -o cpp/test_ring && cpp/test_ring
```

Run each configuration more than once and compare medians within the same
session; do not compare against numbers from another day or another machine.
