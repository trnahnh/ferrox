# Enhancement Report — Ferrox, 2026-10-10

Branch: `enhance/verification-and-benchmarks` (7 commits on top of `main` at `079a748`). `main` untouched.

Machine for every number in this report: AMD Ryzen 7 7735HS (8C/16T), Windows 11, rustc 1.93.1 (LLVM 21.1.8), g++ 14.2.0 (MSYS2), threads unpinned. All numbers were produced by the commands listed under each result; none is estimated or rounded beyond the precision the tool printed.

## 1. Audit

### Architecture

The code matches the documentation: `i64` fixed-point prices, arena + intrusive doubly linked lists + `BTreeMap<i64, PriceLevel>` + `HashMap<u64, u32>` order index, SPSC Disruptor-style ring with `CachePadded` free-running cursors and acquire/release ordering, fixed-size little-endian wire protocol, mmap WAL with CRC32, bincode snapshots, 2-thread gateway. Determinism holds (the `HashMap` is only used for lookups, never for ordering). One hot-path heap operation remains, `BTreeMap` node allocation on price-level creation/removal, and the docs say so.

### Every claimed number

| Claim (where) | Reproduces via | Result |
| --- | --- | --- |
| 137 tests, ~0.25 s (README, CLAUDE.md) | `cargo test --features metrics` | **Reproduced**: 137 passed, 0.18 s. |
| Ring: 1.85 ms vs 16.35 ms = 8.8x; 5.51 ms vs 20.03 ms = 3.6x; 540M / 182M ops/s (README, METRICS) | `cargo bench --bench ring_bench` | **Not reproduced.** Today 3.06 vs 11.80 ms (3.9x; 3.2x vs the faster try_recv variant) and 12.79 vs 14.92 ms (1.17x). The criterion data already on disk from the last run in March (2.45 ms, 10.24 ms) never matched the documented 1.85 / 5.51 ms either. The documented values came from an unpreserved earlier session. |
| Push+pop 2.11 ns (README) | same | Reproduced within noise: 2.37 ns (criterion), 2.3–4.0 ns (new harness). |
| e2e 34 µs / 405 µs / 6.4 ms / 5.6 ms (METRICS) | `cargo bench --bench e2e_bench` | Stored criterion data matches the docs exactly (33.46 µs, 404 µs, 6.42 ms, 5.56 ms). Today 1.2–1.7x slower across the board (49.1 µs, 681 µs, 9.69 ms, 6.75 ms); the whole machine is slower this session, cause not identified. |
| P50/P90/P99/P99.9 = 100/200/500/800 ns, mean 151 ns, 4.7M orders/s (README headline, METRICS, PHASES) | `cargo run --release --example loadgen --features metrics` | **Reproduced in shape, not in value, and the method was flawed.** `std::time::Instant` on this machine advances in 100 ns steps (5M consecutive reads: every non-zero delta was a multiple of 100 ns, 1,494,107 of them exactly 100 ns), so every published percentile was quantized to that grid. With the original binary today: 200/400/700/1800 ns, 2.9–3.05M/s. With a TSC clock (new): P99 832 / 1062 / 982 ns over three runs. Throughput without timing overhead: 4.41M and 4.36M/s, close to the documented 4.7M. |
| "6 allocations, from BTreeMap level management" (METRICS Phase 7) | `cargo run --release --example loadgen --features metrics,alloc-track` | **Count right, attribution unverified and partly wrong.** The histogram was allocated after tracking started: 6 allocs, 223,120 bytes. Engine alone: 5 allocs, 5 deallocs, 1,936 bytes, identical with and without `metrics`. That is BTreeMap nodes. Fixed in `loadgen`. |
| "6 unsafe blocks (4 ring, 2 WAL)" (README) | `grep -n unsafe src/` | **Understated.** Ring had 3 blocks + 3 `unsafe impl`; WAL 2 blocks; `alloc_track.rs` 2 blocks + 1 `unsafe impl`, compiled unconditionally. Corrected inventory in `docs/VERIFICATION.md §3`. |
| "Acquire/Release compiles to plain MOV on x86, zero barrier overhead" (README) | `bash scripts/asm_ordering.sh` | **Verified** with the generated code, Rust and C++: release stores are `movq`, zero `xchg`/`mfence`/`lock`; SeqCst gives `xchgq`. |
| WAL 56 ns, snapshot 126 µs, recovery 1.4 ms (METRICS Phase 6) | `cargo bench --bench wal_bench` | Stored criterion data matches (55.99 ns, 55+72 µs, 1.39 ms). Not re-run today. |
| Phase 1→2→3 before/after tables (METRICS) | `git checkout f019ec3 / 0e3da15 / 5d28e69 && cargo bench --bench matching_bench` | Reproducible only from history; the Phase 1 and Phase 2 implementations no longer exist in HEAD. Not re-run. Documented as the reproduction path. |
| Loadgen workload "crossing orders" (README Phase 7) | loadgen prints "Fills produced" | 628,451 of 990,000 bids fill; 361,549 rest. It is a 63/37 mix, not all-crossing. Documented. |
| CLAUDE.md (gitignored): 9.6x, 520M, 3.9x, 2.13 ns | n/a | Local file disagrees with README (8.8x, 540M, 3.6x, 2.11 ns). Noted; Phase 8 section added. |
| Cloud deployment cost/cold-start (CLOUD_DEPLOYMENT.md) | AWS | Not re-run (infra torn down; would cost money). Out of scope. |

### Flags

1. Benchmarks on this laptop vary 1.3–1.7x between sessions and up to 1.4x between runs of the same binary in one session. Every doc now carries its session date and the rule "compare within a session".
2. The two headline numbers (500 ns P99, 8.8x) were the two least reproducible. Both are replaced by what reproduces today, with the history kept.
3. No bug was found in the matching logic or the ring. The differential tests and loom agree with the implementation.

## 2. Options considered and ranked

Ranked by (keyword value × credibility) / effort. Effort is the estimate made before starting; "spent" is actual.

| # | Option | Est. h | Spent (approx.) | 5 keywords earned | Status |
| --- | --- | --- | --- | --- | --- |
| 1 | loom model-check of the ring + negative control | 2 | 2.5 | lock-free, C11 memory model, model checking, data race, happens-before | **Done** |
| 2 | Memory ordering + cache analysis (generated asm; SeqCst / false sharing / cursor cache / bounds check knobs) | 2 | 2 | acquire/release, x86 TSO, false sharing, cache coherence, codegen | **Done** |
| 3 | C++20 port of the ring, tests, like-for-like harness | 3 | 2.5 | C++, std::atomic, alignas, RAII/templates, cross-language benchmarking | **Done** |
| 4 | Differential property tests (engine vs naive model; ring vs deque) | 1.5 | 1.5 | property-based testing, differential testing, test oracle, determinism, invariants | **Done** |
| 5 | Fix latency measurement (TSC clock, visible resolution) | 1 | 1 | tail latency, clock resolution, HdrHistogram, measurement methodology, observability | **Done** |
| 6 | Sanitizers: Miri, TSan (Rust), TSan/ASan/UBSan (C++) | 1 (+CI) | 0.5 | memory safety, data race detection, undefined behavior, sanitizers, CI | **Written as CI, not run** (no Linux toolchain on this host; installing one is outside the repo) |
| 7 | C++ order-book core (arena + intrusive list + `std::map`) vs Rust | 5 | — | C++, red-black tree vs B-tree, allocator, cache locality, cross-language | Not done: compares `std::map` to `BTreeMap` as much as C++ to Rust; lower marginal keyword gain per hour than 1–5 |
| 8 | Thread pinning / isolated-core harness to cut variance | 2 | — | CPU affinity, NUMA, jitter, benchmarking methodology, tail latency | Not done; would make the numbers in §1 tighter. Recommended next |
| 9 | Hardware counters (cache misses, coherence traffic) | 2 | — | perf, PMU, cache misses, MESI, profiling | Not possible here (no `perf`/VTune on this Windows box) |

Items 1–6 were executed in this session; hours in the table are estimates, not timed.

## 3. Results

Each result: what changed, before/after, how to reproduce, keywords, and a three-sentence link to a Google-scale problem.

### R1. The ring is model-checked under the C11 memory model (loom)

**Change.** `src/ring.rs` routes slot access through a `Slot<T>` wrapper so it compiles against loom's instrumented atomics (`--cfg loom`). `tests/loom_ring.rs` checks FIFO across wraparound and backpressure and exactly-once drop. A `--cfg ring_relaxed_ordering` knob (loom-only; `compile_error!` otherwise) is the negative control.

**Before:** correctness argued in `// SAFETY` comments and two 100k-item concurrent unit tests on one machine. **After:**

```text
RUSTFLAGS="--cfg loom" cargo test --release --test loom_ring
  4 passed; finished in 5.42s          (preemption bound 3; unbounded did not finish in 10 min)

RUSTFLAGS="--cfg loom --cfg ring_relaxed_ordering" cargo test --release --test loom_ring
  Causality violation: Concurrent read and write accesses.   (3 FIFO tests FAILED;
  drop test: STATUS_ACCESS_VIOLATION)
```

The default build is unchanged: push+pop instruction stream identical before and after the refactor (64 instructions).

**Keywords:** lock-free, acquire/release, C11 memory model, model checking, data race, happens-before.

**Google scale.** A lock-free queue that is correct on one laptop can still have an interleaving that only appears once per billion operations, and at fleet scale a billion operations is a few seconds. Model checking explores those interleavings before deployment instead of waiting for a one-in-a-billion crash to page someone. The negative control matters as much as the pass: it shows the check has teeth and that the acquire/release pair is load-bearing, not cargo cult.

### R2. Acquire/release vs SeqCst, false sharing, cursor caching: measured, not asserted

**Change.** Compile-time knobs in both the Rust ring and the C++ port; `scripts/asm_ordering.sh` emits the real x86-64 code into `docs/asm/`.

**Codegen** (both languages): release stores `movq %r8, 64(%r10)` / `movq %r8, 128(%r9)`, zero fences; SeqCst `xchgq` on each cursor store.

**Runtime** (median of 30 × 1M items, same session):

| variant | Rust u64 | Rust Order48 | C++ u64 | C++ Order48 |
| --- | --- | --- | --- | --- |
| shipped | 4.419 ms | 13.206 ms | 5.174 ms | 16.387 ms |
| SeqCst | 23.647 (5.4x) | 30.507 (2.3x) | 30.959 (6.0x) | 34.188 (2.1x) |
| no padding (false sharing) | 9.091 (2.1x) | 15.597 (1.2x) | 9.092 (1.8x) | 15.580 (1.0x) |
| no cursor cache | 7.508 (1.7x) | 13.014 (1.0x) | 7.051 (1.4x) | 16.850 (1.0x) |

**Reproduce:** `bash scripts/asm_ordering.sh`; `RUSTFLAGS="--cfg ring_seqcst" cargo run --release --example ring_throughput -- 30` (and `ring_no_padding`, `ring_no_cursor_cache`); `g++ -std=c++20 -O3 -pthread -DFERROX_SEQ_CST cpp/bench_ring.cpp -o cpp/bench_ring && cpp/bench_ring 30` (and `FERROX_NO_PADDING`, `FERROX_NO_CURSOR_CACHE`). Full tables: `docs/MEMORY_ORDERING.md §1–2`.

**Keywords:** memory ordering, x86 TSO, false sharing, cache coherence, store buffer, codegen.

**Google scale.** Over-synchronizing is the safe default and it cost 5–6x here; at the scale of a datacenter message bus that is the difference between one rack and five. False sharing doubled transfer time without changing a single line of logic, which is why cache-line layout is a design input and not a tuning afterthought. Having the generated instructions in the repo turns a performance belief into something a reviewer can check in thirty seconds.

### R3. C++ port of the ring, compared like-for-like

**Change.** `cpp/spsc_ring.hpp` (C++20, `std::atomic`, `alignas(64)`, `std::launder`, same knobs as macros), `cpp/test_ring.cpp` (6 tests mirroring the Rust ones), `cpp/bench_ring.cpp` and `examples/ring_throughput.rs` sharing one methodology.

**Result** (two interleaved runs each, median of 30):

| | Rust | C++ |
| --- | --- | --- |
| u64 / 1M, median | 4.419, 4.751 ms | 5.174, 5.417 ms |
| Order48 / 1M, median | 13.206, 14.280 ms | 16.387, 17.125 ms |
| Order48 / 1M, min | 5.985, 6.712 ms | 5.539, 5.467 ms |
| push+pop single-thread | 4.044, 2.806 ns | 1.138, 1.847 ns |

Cross-thread: a tie within noise (medians favor Rust 10–20%, minimums favor C++ 5–10%, both inside the run-to-run spread). **Single-threaded, C++ is 1.5–3.5x faster, consistently.** That is the negative result. The bounds check accounts for 10–25% of the Rust loop (R6); the remainder is not explained here (candidates: `Result` return convention vs out-parameter, GCC vs LLVM loop scheduling). It does not affect the two-thread case, where the bottleneck is cache-line transfer.

**Reproduce:** `cargo run --release --example ring_throughput -- 30`; `g++ -std=c++20 -O3 -pthread cpp/bench_ring.cpp -o cpp/bench_ring && cpp/bench_ring 30`; `g++ -std=c++20 -O2 -pthread cpp/test_ring.cpp -o cpp/test_ring && cpp/test_ring`.

**Keywords:** C++, std::atomic, memory_order_acquire/release, alignas, RAII, cross-language benchmarking.

**Google scale.** Most of the latency-critical infrastructure a systems intern would touch is C++, so the useful skill is not "Rust is fast" but being able to write the same lock-free algorithm in both, keep the layout identical, and measure where the languages actually differ. Here the answer is: nowhere that matters across cores, and measurably in a single-core micro-loop, which is the kind of specific claim a design review can act on. Reporting the loss is the point; a comparison that only ever favors the author's language is not a comparison.

### R4. Differential property tests against a naive specification

**Change.** `tests/book_model.rs`: random 1–200-step add/cancel sequences through the engine and a flat-`Vec` reference book; agreement required after every step on status, fills, cancels, best bid/ask, count and full canonical contents; plus a replay-determinism property. `tests/ring_model.rs`: ring vs bounded `VecDeque`, capacities 1–64.

**Before:** 5 proptests on one- and two-order scenarios. **After:** 512 random sequences per property, hundreds of interleaved operations each, no disagreement found. 137 → 141 tests.

**Reproduce:** `cargo test --test book_model --test ring_model` (2.5 s).

**Keywords:** property-based testing, differential testing, test oracle, invariants, deterministic replay.

**Google scale.** When a system handles money or billing, "the fast implementation agrees with the obvious one on every input we can generate" is the correctness standard, and a flat-vector oracle is the cheapest way to state the spec. Differential testing is how production-grade compilers, storage engines and serializers get checked at Google scale, because hand-written cases never reach the interleavings that matter. The determinism property is the same contract that makes replicated state machines and crash recovery possible.

### R5. Latency measurement that shows its own resolution

**Change.** `examples/loadgen.rs` times each order with the x86-64 TSC (calibrated against `Instant`, 0.3 ns step) and prints the observed clock step; `--instant` keeps the old method. The histogram is created before allocation tracking starts.

**Before/after on the same machine, same session:**

| clock | P50 | P90 | P99 | P99.9 | mean | throughput |
| --- | --- | --- | --- | --- | --- | --- |
| `Instant`, 100 ns step (published method) | 200 | 400–500 | 700–1100 | 1600–1800 | 252–291 | 2.63–3.05M/s |
| TSC, run 1 / 2 / 3 | 130 / 210 / 190 | 321 / 481 / 431 | 832 / 1062 / 982 | 1404 / 1584 / 1684 | 171 / 257 / 250 | 4.01 / 3.23 / 3.21M/s |
| no timing | — | — | — | — | — | 4.41, 4.36M/s |

Allocation tracking: 6 allocs / 223,120 B (histogram counted) → 5 allocs / 1,936 B (engine only).

**Reproduce:** `cargo run --release --example loadgen --features metrics` (TSC), `-- --instant`, `cargo run --release --example loadgen` (no timing), `--features metrics,alloc-track`.

**Keywords:** tail latency, clock resolution, TSC, HdrHistogram, measurement methodology, observability.

**Google scale.** SRE dashboards are only as honest as the clock under them; a histogram whose buckets are coarser than the thing being measured will report a clean 500 ns P99 that is really "somewhere between 400 and 600". Printing the clock's own step next to the percentiles makes the limitation visible to whoever reads the dashboard next. The same discipline applies to any latency SLO: state the resolution, state the overhead, and separate the measured throughput from the unmeasured one.

### R6. The price of a bounds check, quantified

**Change.** `--cfg ring_unchecked_index` swaps the safe `buffer[cursor & mask]` for `get_unchecked`.

**Result** (three alternating runs): single-thread push+pop 2.653/3.420/2.840 ns → 2.296/2.614/2.478 ns (10–25% faster); cross-thread u64 and Order48: no gain (the unchecked build was slower in all three runs, within noise). Decision: keep the bounds check; it is free where the ring runs.

**Reproduce:** `RUSTFLAGS="--cfg ring_unchecked_index" cargo run --release --example ring_throughput -- 30` vs the default.

**Keywords:** memory safety, bounds checking, zero-cost abstraction, unsafe Rust, microbenchmarking.

**Google scale.** The recurring argument against memory-safe languages in hot paths is the cost of the checks, and the useful response is a number, not a position. Here the check is 10–25% of a 2.5 ns single-core loop and invisible once a second core is involved, so the safe code stays. Decisions like this, made per hot path with a measurement attached, are how a large codebase moves to safer defaults without losing its latency budget.

### R7. Sanitizers and Miri (written, not run)

**Change.** `.github/workflows/verify.yml`: Miri on the ring's unsafe slot code; ThreadSanitizer on the Rust ring (`-Zsanitizer=thread -Zbuild-std`); TSan then ASan+UBSan on the C++ port; loom; and two negative controls (loom and TSan on the `Relaxed` variants must fail or the job fails).

**Status: not executed.** This Windows host has no sanitizer-capable toolchain (Rust TSan is Linux-only, MinGW g++ ships no sanitizer runtime, WSL Ubuntu has no compilers), and installing one is outside the repository. The first push of the branch runs it; a red job is a finding to record.

**Keywords:** ThreadSanitizer, AddressSanitizer, UndefinedBehaviorSanitizer, Miri, continuous integration.

**Google scale.** Dynamic race and memory-error detection in CI is table stakes for C++ at scale because review cannot see data races and tests cannot reliably trigger them. The negative controls are the part most CI setups skip: a sanitizer job that never fails is indistinguishable from one that is misconfigured. Honest status (written, unverified) beats a green badge nobody has seen.

## 4. What changed, by file

| Path | Change |
| --- | --- |
| `src/ring.rs` | `Slot<T>` wrapper; loom/std import switch; `STORE_ORDER`/`LOAD_ORDER` consts; cfg knobs; module docs. Default codegen identical. |
| `Cargo.toml`, `Cargo.lock` | `loom` under `[target.'cfg(loom)'.dependencies]`; `check-cfg` declarations; `ring_throughput` example. |
| `tests/loom_ring.rs`, `tests/book_model.rs`, `tests/ring_model.rs` | New. |
| `cpp/spsc_ring.hpp`, `cpp/test_ring.cpp`, `cpp/bench_ring.cpp`, `cpp/asm_probe.cpp` | New. |
| `examples/ring_throughput.rs` | New. |
| `examples/loadgen.rs` | TSC clock, `--instant`, clock-step print, resting/fill split, histogram before tracking. |
| `scripts/asm_ordering.sh`, `scripts/asm_probe.rs`, `docs/asm/*.s` | New. |
| `.github/workflows/verify.yml` | New (not run). |
| `README.md`, `docs/METRICS.md`, `docs/PHASES.md` | Numbers replaced with reproducible ones, history kept; Phase 8 added. |
| `docs/VERIFICATION.md`, `docs/MEMORY_ORDERING.md` | New. |
| `.gitignore` | C++ build outputs. |

No files deleted. No force pushes. Nothing modified outside the repository (CLAUDE.md, which is gitignored, gained a Phase 8 section).

## 5. Keywords earned (general)

lock-free data structures · C11/C++ memory model (acquire/release vs SeqCst) · model checking (loom) · cache coherence and false sharing · tail-latency measurement methodology · differential / property-based testing · memory safety (bounds checks, Miri, sanitizers) · cross-language systems benchmarking (Rust / C++)

## 6. Recommended next steps (not done)

1. Push the branch and read the `verify` workflow results; fix or record whatever fails.
2. Pin the producer and consumer to two physical cores in `ring_throughput` (`SetThreadAffinityMask` / `sched_setaffinity`) to shrink the run-to-run spread that currently hides the Rust-vs-C++ cross-thread answer.
3. Investigate the single-threaded Rust-vs-C++ gap with the probe assembly (`docs/asm/`) extended to the full push+pop loop; the `Result` return convention is the first suspect.
4. Re-run `matching_bench` at `f019ec3`, `0e3da15`, `5d28e69` in one session to refresh the Phase 1→2→3 tables with same-session numbers.
