# Verification

How the two pieces of Ferrox that are easiest to get subtly wrong, the lock-free
ring and the matching logic, are checked beyond unit tests. Everything in
sections 1–3 was run on 2026-10-10 on Windows; section 4 is a CI workflow that
needs Linux and has not been executed yet.

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
| `src/ring.rs` | 6 | 3 | 3 primitive slot operations (`write`, `read`, `drop_in_place`) each in an `unsafe fn` on `Slot<T>` with its contract, plus the 3 call sites in `push`, `pop`, `Drop`; `Send`/`Send`/`Sync` for producer/consumer/inner. One more block exists only under `--cfg ring_unchecked_index`. |
| `src/wal.rs` | 2 | 0 | `MmapMut::map_mut` |
| `src/alloc_track.rs` | 2 | 1 | delegation to `System` in the tracking `GlobalAlloc` (compiled unconditionally; only installed as the global allocator by `loadgen` under `--features alloc-track`) |

The slot operations are what loom checks for races and what the Miri job
below checks for undefined behaviour.

## 4. Sanitizers and Miri (CI, not yet run)

ThreadSanitizer, AddressSanitizer, UndefinedBehaviorSanitizer and Miri all
need a Linux toolchain (Rust TSan is Linux-only; MinGW g++ ships no sanitizer
runtime; WSL on the dev machine has no compilers installed). Rather than
install toolchains outside the repository, the checks are a GitHub Actions
workflow, [`.github/workflows/verify.yml`](../.github/workflows/verify.yml):

| job | command | purpose |
| --- | --- | --- |
| `miri` | `cargo miri test --lib ring:: -- --skip concurrent_` | interpret the unsafe slot code; flags uninitialized reads, invalid pointers, double drops |
| `tsan-rust` | `RUSTFLAGS=-Zsanitizer=thread cargo test -Zbuild-std --lib ring::` | dynamic data-race detection on the real concurrent unit tests |
| `cpp-sanitizers` | `clang++ -fsanitize=thread`, then `-fsanitize=address,undefined` on `cpp/test_ring.cpp` | same for the C++ port |
| negative controls | loom and TSan on the `Relaxed` variants | the workflow fails if either tool stops detecting the race |

**Status: written on Windows, never executed.** The first push of this branch
is its first run; any failure there is a finding to record, not something to
paper over.

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
| `killed_writer_recovers_to_a_consistent_prefix` | a writer process (this test binary re-executed) appends and snapshots; it is `kill`ed at a varying point, recovery runs, the writer is restarted and resumes from the recovered state; 12 cycles | 12 kills, 2,368 records | 0 failures |

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

Two observations that are not defects:

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

Not covered, and therefore not claimed: durability across *power loss*. The WAL
is an `mmap` with no `msync` ordering and the snapshot does no `fsync` before
`rename`, so a kernel crash or power cut can lose or reorder page writeback in
ways a process kill cannot. The harness proves process-crash consistency only.

```text
$ cargo test --release --test crash_recovery -- --nocapture --test-threads=1
wal truncation: 625 cases, 0 failures
wal corruption: 1548 cases, 0 failures
snapshot corruption: 1189 cases, 0 failures
process-kill: 12 cycles, 2368 records total, ...
test result: ok. 5 passed; finished in 54.61s        (Windows, release)
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
