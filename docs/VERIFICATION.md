# Verification

How the three pieces of Ferrox that are easiest to get subtly wrong, the
lock-free ring, the matching logic and the crash-recovery path, are checked
beyond unit tests. Sections 1–3 were run on 2026-10-10 on Windows; section 4
(sanitizers, Miri) was run on 2026-10-11 in WSL2 Ubuntu; section 5 (fault
injection) on 2026-10-11 on Windows and Linux.

## 1. Model checking the ring with loom

[loom](https://docs.rs/loom) runs a closure under every thread interleaving the
C11 memory model permits, including stale reads that acquire/release allows,
and checks every `UnsafeCell` access for a data race. Compiling with
`--cfg loom` swaps the ring's `AtomicUsize`, `UnsafeCell` and `Arc` for loom's
instrumented versions; nothing else in the ring changes, and the default build
is byte-for-byte the same code (verified via `--emit asm`, see
[MEMORY_ORDERING.md §1](MEMORY_ORDERING.md#1-what-the-compiler-emits-acquirerelease-vs-seqcst)).

`tests/loom_ring.rs` checks:

| test | what it exercises |
| --- | --- |
| `fifo_capacity_1_two_items` | every push after the first must observe the consumer's release of the slot |
| `fifo_capacity_2_three_items` | one wraparound: slot 0 is rewritten while the consumer may still be reading it |
| `fifo_capacity_2_five_items` | repeated wraparound under sustained backpressure |
| `unconsumed_items_are_dropped_exactly_once` | `Drop` of the ring with items left in it: no leak, no double drop |

Exhaustive exploration of the spin-wait loops did not finish in 10 minutes
(killed), so the tests use loom's preemption bound of 3, the CHESS-style bound
loom recommends; most real concurrency bugs need two or fewer preemptions.

```text
$ RUSTFLAGS="--cfg loom" cargo test --release --test loom_ring
test fifo_capacity_1_two_items ... ok
test fifo_capacity_2_five_items ... ok
test fifo_capacity_2_three_items ... ok
test unconsumed_items_are_dropped_exactly_once ... ok
test result: ok. 4 passed; 0 failed; finished in 5.42s
```

**Negative control.** A checker that never fails proves nothing, so the ring
has a `--cfg ring_relaxed_ordering` knob that downgrades every cursor access to
`Relaxed` (it refuses to compile without `--cfg loom`). Loom catches it on
every FIFO test, and the drop test crashes outright because the consumer reads
a slot whose contents are not yet visible:

```text
$ RUSTFLAGS="--cfg loom --cfg ring_relaxed_ordering" cargo test --release --test loom_ring
thread 'fifo_capacity_1_two_items' panicked at loom-0.7.2/src/rt/location.rs:115:9:
Causality violation: Concurrent read and write accesses.
test fifo_capacity_1_two_items ... FAILED
test fifo_capacity_2_five_items ... FAILED
test fifo_capacity_2_three_items ... FAILED
test unconsumed_items_are_dropped_exactly_once ... (process exited: 0xc0000005, STATUS_ACCESS_VIOLATION)
```

This is the whole argument for the `Release`/`Acquire` pair in six lines of
output: with it, no interleaving within the bound can observe a half-written
slot; without it, the first interleaving loom tries does.

## 2. Differential testing of the matching engine

`tests/book_model.rs` runs random sequences of 1–200 adds and cancels (prices in
a 11-tick band, four trader ids so self-trades happen, quantities 1–20) through
two implementations at once:

- the real `MatchingEngine` (arena, intrusive lists, `BTreeMap`, cached best
  prices), and
- a reference book that is a flat `Vec<Order>` scanned linearly for the best
  maker on every step, with no caches and no data-structure cleverness.

After **every** operation the two must agree on the order status, the exact
fill list, the cancel result, best bid, best ask, resting count, and the full
canonical book contents (`all_resting_orders`). A second property replays the
same sequence twice and requires identical transcripts (the determinism hard
rule). The reference itself is pinned by a hand-checked scenario so a bug in
the oracle cannot silently agree with a bug in the engine.

`tests/ring_model.rs` does the same for the ring single-threaded against a
bounded `VecDeque` across capacities 1–64: `Full`/`Empty` must be reported
exactly when the model says, `Full` must hand the value back, and cursor
wraparound must never reorder.

```text
$ cargo test --test book_model --test ring_model
test reference_model_sanity ... ok
test replay_is_deterministic ... ok
test engine_matches_reference_model ... ok
test ring_matches_bounded_deque ... ok
```

512 cases per property. No disagreement was found; the existing engine is
correct on everything these tests can reach. Total suite after this work: 141
tests with `--features metrics` (137 before), plus 4 loom tests under `--cfg loom`
and 6 C++ tests in `cpp/test_ring.cpp`.

## 3. `unsafe` inventory

The README's "6 unsafe blocks" undercounted. Actual inventory of the default
(non-loom) build after this work:

| file | `unsafe` blocks | `unsafe impl` | what they do |
| --- | --- | --- | --- |
| `src/ring.rs` | 7 | 3 | 3 primitive slot operations (`write`, `read`, `drop_in_place`) each in an `unsafe fn` on `Slot<T>` with its contract, plus the 3 call sites in `push`, `pop`, `Drop`; `ManuallyDrop::take` in `Shared::drop` (runs once, by construction of `Drop`); `Send`/`Send`/`Sync` for producer/consumer/inner. One more block exists only under `--cfg ring_unchecked_index`. |
| `src/wal.rs` | 2 | 0 | `MmapMut::map_mut` |
| `src/alloc_track.rs` | 2 | 1 | delegation to `System` in the tracking `GlobalAlloc` (compiled unconditionally; only installed as the global allocator by `loadgen` under `--features alloc-track`) |

The slot operations are what loom checks for races and what the Miri job
below checks for undefined behaviour.

## 4. Sanitizers and Miri (run on Linux, 2026-10-11)

ThreadSanitizer, AddressSanitizer, UndefinedBehaviorSanitizer and Miri need a
Linux toolchain (Rust TSan is Linux-only; MinGW g++ ships no sanitizer
runtime). Round 1 left them as an unexecuted CI workflow. Round 2 installed
the toolchain inside WSL2 Ubuntu 24.04 (gcc 13.3, clang 18.1.3, rustup
stable 1.99 + nightly 1.101 with miri/rust-src) and ran them by hand;
[`scripts/wsl_queue.sh`](../scripts/wsl_queue.sh) is the exact sequence and
[`.github/workflows/verify.yml`](../.github/workflows/verify.yml) mirrors it.
The VM has 3.9 GB shared with Docker Desktop containers that held ~3.5 GB,
so every cargo build there swapped; which steps completed is recorded
honestly below and in [ROUND2_REPORT.md](../ROUND2_REPORT.md).

| check | command (see queue script) | result |
| --- | --- | --- |
| C++ ThreadSanitizer | `clang++ -std=c++20 -O1 -g -fsanitize=thread -pthread cpp/test_ring.cpp` | **6 tests ok, no report, exit 0** |
| C++ ASan + UBSan | `clang++ … -fsanitize=address,undefined -fno-omit-frame-pointer` | **6 tests ok, no report, exit 0** |
| C++ TSan negative control | same, `-DFERROX_RELAXED_UNSOUND` | **caught**: `WARNING: ThreadSanitizer: data race … spsc_ring.hpp:175:13 in Consumer::pop` (the slot read racing the producer's slot write), 2 reports, exit 66 |
| Rust Miri (ring, single-threaded tests) | `cd verify/ring-standalone && MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly miri test -- --skip concurrent_` | see ROUND2_REPORT.md §3 (filled in from the queue log) |
| Rust ThreadSanitizer (ring, concurrent tests) | `cd verify/ring-standalone && RUSTFLAGS=-Zsanitizer=thread cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu` | see ROUND2_REPORT.md §3 |
| Rust TSan negative control | same with `--cfg ring_relaxed_ordering --cfg ring_allow_unsound`, must print `ThreadSanitizer: data race` | see ROUND2_REPORT.md §3 |
| loom on Linux + negative control | `RUSTFLAGS="--cfg loom" cargo test --release --test loom_ring` | see ROUND2_REPORT.md §3 (passes on Windows, §1) |

Why `verify/ring-standalone` exists: `-Zbuild-std` rebuilds the standard
library with TSan and Miri interprets every instruction, so both are run on a
crate that contains nothing but `src/ring.rs` (included by `#[path]`, not
copied) rather than on the main crate with criterion and proptest behind it.
It is the same module and the same 13 tests.

One operational note from running this for real: the exit code of a
sanitizer run only means something when read in the same shell that ran it.
Driving WSL from a Windows shell expanded `$?` on the Windows side, which made
the Relaxed build look like it exited 0; a script file run inside WSL gave the
true codes (66 with a race, 0 without). The CI step relies on that exit code
and is therefore correct, but it was checked rather than assumed.

## 5. Crash consistency: fault injection of the WAL and snapshots (2026-10-11)

Phase 6 claimed "truncate WAL at random points, verify correct recovery". That
was one hand-written unit test per failure mode. `tests/crash_recovery.rs`
replaces it with an exhaustive harness built on three rules a recovery must
satisfy after *any* crash: never panic; yield exactly the longest valid prefix
of the log; and produce the same book as replaying that prefix into a fresh
engine, whether or not a snapshot was used.

The command stream is a pure function of `(seed, index)` (splitmix-style hash:
~75% new orders in an 11-tick band across four trader ids, ~25% cancels of
earlier ids), so any process can regenerate exactly what another one wrote.

| test | faults injected | cases | result |
| --- | --- | --- | --- |
| `wal_truncated_at_every_byte_offset_…` | log cut at every byte offset 0..=624 of a 16-record log; after recovery, 3 more records are appended, the process exits, and recovery runs again | 625 | 0 failures |
| `wal_corrupted_at_every_byte_offset_…` | byte xor 0xFF, byte zeroed, bit 0 flipped, at every offset, each followed by the same append-and-recover-again step | 1,548 | **698 failures before the fix**, 0 after |
| `snapshot_corrupted_at_every_byte_offset_…` | the same three mutations at every offset of a 566-byte snapshot, with a 40-record WAL whose first 25 records the snapshot covers | 1,189 | **16 failures before the fix**, 0 after |
| `killed_writer_recovers_to_a_consistent_prefix` | a writer process (this test binary re-executed) appends and snapshots; it is `kill`ed at a varying point, recovery runs, the writer is restarted and resumes from the recovered state; 12 cycles | 12 kills, 92,525 records (the writer runs until killed; an earlier 50,000-record cap let it finish early on a quiet machine, see the note below the table) | 0 failures |

Two real defects, both fixed in this branch:

1. **Stale records resurrected after a corrupt record** (`src/wal.rs`).
   Recovery stopped at the first bad record and set the write position there,
   but left the bytes after it untouched. Those bytes still held the *old*
   records. New appends overwrote them from the front; as soon as the new
   records realigned with the old record boundaries (immediately, since every
   `NewOrder` record is 48 bytes), the next recovery read straight through the
   new records into the old ones and brought back history the previous
   recovery had declared lost. The harness shows it as "after appending 3,
   recovered 16 records, expected 3". Fix: `Wal::open` truncates the file to
   the end of the valid prefix and re-extends it, which costs two metadata
   syscalls and makes everything past `write_pos` read as zero, the invariant
   the rest of the module now relies on.
2. **Snapshot checksum did not cover its own header** (`src/snapshot.rs`). The
   CRC32 was computed over `orders` only, so a corrupted `wal_record_count`
   passed verification and replay started from the wrong record: 16 cases,
   exactly the bytes 0–7 of the file. Fix: the checksum covers every field.
   Corruption of `best_bid`/`best_ask` was harmless (restore recomputes them)
   and is now also detected.

Three observations that are not defects:

- Cutting a record inside its trailing zero bytes (the high bytes of a small
  quantity) yields a file that is byte-identical to the intact one after the
  WAL zero-extends it, so the record correctly survives. 112 of the 625
  truncation offsets are of this kind; the oracle compares bytes, not lengths.
- On Windows the first version of the kill test landed every kill inside
  `Snapshot::save` (the record count was a multiple of the snapshot interval
  in all 12 cycles) because `fs::write` + `rename` dominates the writer's wall
  time there. The interval was raised so kills land in both regions; the test
  prints how many hit each. Each kill during a save leaves a
  `snapshot_*.bin.tmp` behind, which the loader ignores by name; they are not
  cleaned up.
- The writer child originally stopped after 50,000 records. With the host
  otherwise idle it reached that cap in 8 of the 12 cycles, the ninth child
  had nothing to write, and the parent timed out waiting for an
  acknowledgement (2026-10-11, after the WSL VM had stopped). The child now
  runs until killed; the per-cycle record counts depend on machine speed and
  are printed, not asserted.

Not covered, and therefore not claimed: durability across *power loss*. The WAL
is an `mmap` with no `msync` ordering and the snapshot does no `fsync` before
`rename`, so a kernel crash or power cut can lose or reorder page writeback in
ways a process kill cannot. The harness proves process-crash consistency only.

```text
$ cargo test --release --test crash_recovery -- --nocapture --test-threads=1
wal truncation: 625 cases, 0 failures
wal corruption: 1548 cases, 0 failures
snapshot corruption: 1189 cases, 0 failures
process-kill: 12 cycles, 92525 records total, 7 kills during a snapshot save, 5 between appends, 12 cycles recovered through a snapshot, 7 cycles saw a leftover .tmp snapshot
test result: ok. 5 passed; finished in 43.12s        (Windows, release, 2026-10-11)
```

## 6. Measurement checks that came out of the audit

- `std::time::Instant` on this Windows machine advances in 100 ns steps
  (QueryPerformanceCounter at 10 MHz, measured: 1.49M of 5M consecutive reads
  advanced, every one by exactly 100 ns; a read costs 26 ns). The Phase 7
  percentiles of 100/200/500/800 ns were therefore quantized to that grid.
  `examples/loadgen.rs` now times each order with the x86-64 TSC (0.3 ns step)
  and prints the observed clock step; `--instant` reproduces the old method.
- The allocation tracker started before the latency histogram was created, so
  the "6 allocations" it reported included the histogram's ~220 KB counts
  array. Created in the right order it reports 5 allocations, 5 deallocations,
  1,936 bytes for 990,000 orders, the same with and without `--features
  metrics`, consistent with `BTreeMap` price-level nodes (~390 bytes each).
