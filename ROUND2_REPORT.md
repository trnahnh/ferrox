# Round 2 Report — Ferrox, 2026-10-11

Branch: `round2/ferrox`, created from `enhance/verification-and-benchmarks` (round 1, 7 commits over `main` at `079a748`). `main` untouched, nothing pushed, no files deleted, no force pushes. Nothing outside the repository was modified except inside the WSL2 Ubuntu distro (apt packages, rustup, `~/ferrox-build`, `~/ferrox-target`), which the brief allowed.

Machines: the same AMD Ryzen 7 7735HS laptop as round 1 (Windows 11, rustc 1.93.1, g++ 14.2.0 MSYS2) plus WSL2 Ubuntu 24.04 on it (gcc 13.3, clang 18.1.3, rustc stable 1.99.0 / nightly 1.101.0, perf 6.8.12). Every number below comes from a command listed next to it; none is estimated or rounded beyond what the tool printed. Where a run could not be completed, that is stated rather than filled in.

## 1. Audit delta (what round 1 left weak, and what this round found)

| Round-1 weak spot | What was done | Outcome |
| --- | --- | --- |
| Sanitizers written, never run | Toolchain installed in WSL2; C++ TSan, ASan+UBSan and the Relaxed negative control run; Rust Miri/TSan given a zero-dependency crate so they can run at all on a starved VM | C++: all pass, Relaxed caught (exit 66). Rust: see §3 R5 and §6 |
| Run-to-run variance hid the Rust-vs-C++ cross-thread answer | `--pin` in both harnesses, interleaved ABBA driver, n = 40, bootstrap CIs, Mann-Whitney, Cliff's delta | Settled: 48-byte transfer is 10–17% faster in Rust (p ≤ 0.003, three runs); 8-byte transfer is a tie |
| Unexplained 1.5–3.5x single-thread gap | LLVM IR + three codegen experiments + assembly of the exact loop in both languages | Root cause found (not what round 1 guessed), fixed in `src/ring.rs`; gap 2.3x → 1.35x; the rest is the bounds check |
| No hardware counters | `scripts/perf_counters.sh` (`perf stat` on the four ring variants, WSL2) | See §3 R6 and §6 for what the VM allowed |
| No crash testing | Exhaustive fault-injection harness for WAL and snapshots, plus process kills | **Two real defects found and fixed**; 3,362 faults and 12 kills now pass |

New findings about the code, all from this round:

1. **`Wal::open` resurrected discarded records.** After a corrupt/torn record, recovery stopped there but left the old bytes in place; new appends realigned with old record boundaries and the next recovery read the discarded history back. Fixed (`set_len` down to the valid prefix and back up; invariant: bytes beyond `write_pos` are zero).
2. **`Snapshot` checksum covered `orders` only.** A corrupted `wal_record_count` (bytes 0–7 of the file) was accepted and replay started from the wrong record. Fixed: the checksum covers every field.
3. **`Producer`/`Consumer` cursors lived on the stack in every hot loop** because `Arc::drop_slow(&mut self)` pins the containing struct in memory for LLVM. Fixed with a `ManuallyDrop<Arc<_>>` dropped by value; push/pop codegen unchanged.
4. Observations that are not defects: cutting a record inside trailing zero bytes is indistinguishable from the intact record (112 of 625 offsets) and correctly recovers; a kill during `Snapshot::save` leaves a `.tmp` file the loader ignores and nobody cleans up; the persistence layer makes no power-loss claim (no `msync`/`fsync` ordering) and the harness does not test one; the kill-test child originally stopped at 50,000 records and reached that cap on a quiet machine (cycle 9 of 12 then timed out), so it now runs until killed.
5. Round-1 methodology: two unpinned runs could not resolve a 10–17% effect that n = 40 interleaved runs resolve at p < 0.003. The round-1 "tie" was a limitation of the method, now documented in `docs/METHODOLOGY.md`.

## 2. Options considered and ranked

Ranked by (credibility gain × keyword value) / effort. "Spent" is approximate wall time including waiting on the Linux VM.

| # | Option | Est. h | Spent | 5 keywords earned | Status |
| --- | --- | --- | --- | --- | --- |
| 1 | Crash-consistency fault injection (every offset, kills) | 3 | 3 | fault injection, crash consistency, write-ahead log, idempotent recovery, torn writes | **Done, 2 defects fixed** |
| 2 | Single-thread gap root cause + fix | 2 | 2.5 | LLVM IR, SROA/mem2reg, store-to-load forwarding, escape analysis, codegen | **Done** |
| 3 | Pinning + interleaved statistics driver | 2 | 2 | CPU affinity, bootstrap CI, Mann-Whitney U, effect size, benchmark methodology | **Done** (Windows; Linux in queue) |
| 4 | Linux sanitizers (C++ TSan/ASan/UBSan + negative control) | 1 | 1.5 (toolchain fights) | ThreadSanitizer, AddressSanitizer, UBSan, data race, negative control | **Done** |
| 5 | Linux Rust verification: Miri, TSan, loom; zero-dependency ring crate | 2 | 2+ | Miri, undefined behaviour, `-Zbuild-std`, provenance, CI parity | Crate done; runs queued on a starved VM, see §6 |
| 6 | cargo-fuzz: protocol decoder (+ WAL and snapshot loaders) with coverage | 1.5 | 1 (+ queue) | libFuzzer, coverage-guided fuzzing, round-trip property, parser hardening, corpus | Targets written; runs queued, see §6 |
| 7 | `perf stat` hardware counters per ring variant | 1 | 0.5 (+ queue) | PMU, cache misses, LLC, branch misprediction, IPC | Script done; run queued, see §6 |
| 8 | METHODOLOGY.md | 1 | 1 | clock resolution, noise budget, interleaving, pinning, reproducibility | **Done** |
| 9 | C++ order-book core | 5 | — | — | Not done (same reasoning as round 1: compares `std::map` to `BTreeMap`) |

## 3. Results

### R1. Crash consistency, exhaustively (two defects fixed)

**Change.** `tests/crash_recovery.rs` (5 tests). The command stream is a pure function of `(seed, index)`, so a killed writer can be resumed and checked against a fresh replay. Faults: truncation at every byte offset of a 16-record log; xor-0xFF / zero / flip-bit-0 at every offset, each followed by "append 3 more, recover again"; the same three mutations at every offset of a snapshot file; a child process (this test binary re-executed) killed mid-write for 12 cycles while snapshotting.

**Before / after** (Windows, `cargo test --release --test crash_recovery -- --nocapture`):

| test | cases | before fixes | after fixes |
| --- | --- | --- | --- |
| WAL truncation | 625 | 0 failures (after fixing the oracle, see §1.4) | 0 |
| WAL corruption + continue writing | 1,548 | **698 failures** (16 records recovered where 3–14 were expected: stale records resurrected) | 0 |
| snapshot corruption | 1,189 | **16 failures** (bytes 0–7: wrong `wal_record_count` accepted, state diverged from full replay) | 0 |
| process kill | 12 cycles, 92,525 records | 0 | 0; every cycle: recovered count ∈ [last ack, last ack + 1], state == fresh replay == WAL-only replay |

**Reproduce:** `cargo test --release --test crash_recovery -- --nocapture --test-threads=1` (≈55–75 s on Windows, NTFS temp-dir churn). To see the defects: `git checkout a79fb71 -- src/wal.rs src/snapshot.rs` and run again.

**Keywords:** fault injection, crash consistency, write-ahead log, torn writes, idempotent recovery, deterministic replay.

**Google scale.** A storage layer that survives the three crash cases its author imagined is the normal state of a new system; the cases that take down a fleet are the ones nobody imagined, and enumerating every byte offset is the cheapest way to stop relying on imagination. Both defects here were silent-corruption bugs (recovery succeeded with the wrong state), which is the class that costs the most at scale because nothing pages until the books disagree. The "keep writing after recovery, crash again" step is what found the worse of the two; one recovery is never the whole story in a system that restarts.

### R2. The single-thread gap: root cause, fix, measurement

**Change.** `scripts/asm_loop.sh` emits the exact `run_latency` loop of both harnesses; `rustc --emit llvm-ir` showed the `Producer`/`Consumer` allocas passed by pointer to `Arc::drop_slow` in the panic-cleanup block. Three codegen experiments (default; `panic=abort`; `panic=abort` + leak the ring) isolate the escape: only when `drop_slow` is never called do the fields reach registers. `src/ring.rs` now holds the `Arc` in `ManuallyDrop` and drops it by value (`Shared<T>`); push/pop assembly is unchanged (64 instructions, same fence census), loom passes.

**Before / after** (pinned 2,4, interleaved, n = 40, Windows run 1; `docs/bench/2026-10-11_windows_pinned_2-4_run1.json`):

| | median ns per push+pop | 95% CI | vs fixed Rust | p |
| --- | --- | --- | --- | --- |
| Rust before fix | 2.752 | [2.546, 3.083] | 1.710x [1.552, 1.916] | 1.4e-14 |
| Rust after fix | 1.610 | [1.546, 1.708] | — | — |
| C++ (g++) | 1.196 | [1.159, 1.236] | 0.743x [0.691, 0.781] | 3.9e-10 |

Run 2 (noisier, VM swapping) agrees: before 3.403, after 2.679, unchecked-index 1.909, C++ 1.577 ns. The bounds check (`--cfg ring_unchecked_index`) accounts for most of what remains. Round 1's two guesses, the `Result` return convention and GCC-vs-LLVM scheduling, were both wrong.

**Reproduce:** `bash scripts/asm_loop.sh` (reads `docs/asm/rust_latency_bench_default.s` vs `cpp_latency_bench_g++.s`); `python3 scripts/bench_compare.py --pin 2,4 rust=… rust_before_fix=… cpp_gcc=…` with the before-fix binary built from `git stash`/`a79fb71`.

**Keywords:** LLVM IR, SROA / mem2reg, escape analysis, store-to-load forwarding, register allocation, codegen.

**Google scale.** A 1.7x in a 2-nanosecond loop is invisible in any end-to-end number and still worth understanding, because the same mechanism (a local whose address escapes into a cold call) silently pessimizes hot loops across a codebase, and the diagnosis generalizes: read the IR, find the alloca that reaches a call. The fix changed no algorithm and no memory ordering, only who holds a pointer; that is the typical shape of a real codegen win. Writing down the three experiments matters more than the fix, because the next person can rerun them in a minute instead of guessing for a day.

### R3. Rust vs C++ across threads, settled statistically

**Change.** `--pin P,C` (raw FFI, no new crates), `--raw`, `--workload` in both harnesses; `scripts/bench_compare.py` interleaves variants ABBA over 10 rounds × 4 trials and reports median, bootstrap CI, ratio CI, Mann-Whitney U, Cliff's delta.

**Result** (three Windows runs, cores 2 and 4 are distinct physical cores; the WSL2 VM on the host was swapping during all of them):

| workload | runs where Rust is faster at p < 0.01 and ratio CI excludes 1 | ratios (C++ / Rust) | verdict |
| --- | --- | --- | --- |
| Order48 cross-thread | 3 of 3 | 1.122 [1.074, 1.189]; 1.174 [1.043, 1.323]; 1.106 [1.039, 1.180] | **Rust faster, 10–17%** |
| u64 cross-thread | 1 of 3 (unpinned run only) | 1.029 [0.960, 1.104]; 1.091 [0.940, 1.346]; 1.198 [1.059, 1.377] | tie |
| single-thread push+pop | 0 of 3 (C++ faster in all) | 0.743; 0.589; 0.658 | **C++ faster, 1.35–1.7x** |

Pinning removed migration but not the hypervisor neighbor: run 2 pinned had the widest CIs of the three. Minimums (the least-disturbed trials) tell the same story as medians.

**Reproduce:** `python3 scripts/bench_compare.py --pin 2,4 --rounds 10 --iters 4 rust=target/release/examples/ring_throughput cpp_gcc=cpp/bench_ring` (on Windows, run the driver from WSL; the `.exe` paths work through interop). Raw samples: `docs/bench/*.json`.

**Keywords:** CPU affinity, interleaved A/B benchmarking, bootstrap confidence interval, Mann-Whitney U, Cliff's delta, run-to-run variance.

**Google scale.** Every performance claim in a design review is a comparison, and a comparison without a confidence interval is an anecdote; the interleaved driver is the smallest tool that turns the laptop's 1.4x run-to-run noise into a p-value. The answer flipped from "tie" to "Rust 10–17% faster on 48-byte payloads" without the code changing, which is the clearest demonstration that methodology is part of the result. The honest half is that single-core push+pop still favors C++, and the report says so with the same statistics.

### R4. C++ ring under ThreadSanitizer, ASan, UBSan (run, not just written)

**Change.** None to the code; the round-1 commands were executed in WSL2 with clang 18.1.3.

**Result:**

```text
clang++ -std=c++20 -O1 -g -fsanitize=thread -pthread cpp/test_ring.cpp            → 6 tests ok, exit 0
clang++ -std=c++20 -O1 -g -fsanitize=address,undefined -fno-omit-frame-pointer …  → 6 tests ok, exit 0
clang++ … -fsanitize=thread -DFERROX_RELAXED_UNSOUND …                             → exit 66
  WARNING: ThreadSanitizer: data race
  SUMMARY: ThreadSanitizer: data race cpp/spsc_ring.hpp:175:13 in Consumer::pop   (2 reports)
```

The race TSan reports under `Relaxed` is the consumer's slot read against the producer's placement-new, exactly the access the acquire/release pair orders.

**Reproduce:** the three commands above (Linux); `.github/workflows/verify.yml` job `cpp-sanitizers`.

**Keywords:** ThreadSanitizer, AddressSanitizer, UndefinedBehaviorSanitizer, happens-before, negative control.

**Google scale.** Dynamic race detection is the only tool that sees the bug class code review provably cannot, and at fleet scale a once-per-billion interleaving is a daily event. The negative control is the part teams skip: a sanitizer job that has never been seen to fail is indistinguishable from one that is misconfigured, and this one fails on the right line. Running it also surfaced an operational trap (exit codes read through a Windows shell were wrong) that would have made a green CI badge meaningless.

### R5. Rust under Miri and ThreadSanitizer, loom on Linux

**Change.** `verify/ring-standalone`: a crate with no dependencies that includes `src/ring.rs` by path, so `-Zbuild-std` TSan and Miri process only the ring and `std`. The one ring test that touched `crate::order` now uses a local 48-byte payload. `--cfg ring_allow_unsound` lets the Relaxed variant compile for a Rust TSan negative control (`compile_error!` otherwise). `scripts/wsl_queue.sh` is the exact sequence; `verify.yml` mirrors it.

**Result:** see §6. The WSL2 VM (3.9 GB, ~3.5 GB held by Docker Desktop containers that are not mine to stop) swapped continuously; the rustup nightly install alone was interrupted three times. The queue runs the stable-toolchain steps first (loom + negative control → perf → pinned and unpinned variance → full test suite) and then installs nightly for Miri → TSan → TSan negative control → fuzzing + coverage, one job at a time; the log at `~/ferrox-build/logs/queue.log` is the record. Whatever it had produced by the time this report was finalized is in §6; the rest is unverified, not assumed.

**Reproduce:** `bash scripts/wsl_queue.sh` on any Linux box with rustup, clang, perf and python3; or push the branch and read the `miri` / `tsan-rust` / `loom` jobs.

**Keywords:** Miri, strict provenance, `-Zbuild-std`, sanitizer-instrumented std, CI parity.

**Google scale.** Interpreting unsafe code under Miri is the closest thing Rust has to a proof that the `// SAFETY` comments are true, and running it on the smallest possible crate is how it stays cheap enough to run on every change. A verification step that only works on a 64 GB build server is a step that gets skipped; shrinking it to "the ring and std" is the engineering. Reporting "queued on a starved VM" instead of a green checkmark is the only version of this result that a reviewer can trust.

### R6. Hardware counters and fuzzing (written, queued)

`scripts/perf_counters.sh` runs the u64 workload of the shipped, no-padding, SeqCst and no-cursor-cache builds under `perf stat -r 5` (cycles, instructions, cache references/misses, LLC loads/misses, branches/misses, context switches, migrations). The WSL2 kernel reports `AMD PMU driver` and `perf_event_paranoid = 2`; which events the hypervisor actually virtualizes is only known once the script runs, and `<not supported>` is printed as such.

`fuzz/` has three libFuzzer targets: `protocol_decode` (every prefix of the input through `decode_message` and `decode_execution_report`, plus encode→decode round-trip equality for anything that decodes), `wal_recover` (arbitrary bytes as a WAL; recovery must succeed, bound the record count, and be idempotent), `snapshot_load` (arbitrary bytes as the newest snapshot; recovery must not fail). The queue runs each for 600 s and produces an `llvm-cov` report for the decoder.

**Result:** see §6.

**Keywords:** PMU, cache-miss attribution, coverage-guided fuzzing, libFuzzer, parser round-trip property.

**Google scale.** Counters are how "false sharing costs 2x" becomes "false sharing costs 2x because LLC misses tripled", which is the form that transfers to the next design. Fuzzing a wire decoder for ten minutes is cheap insurance against the one malformed packet a billion that a hand-written test never contains. Both are written so that the next machine with a PMU and a quiet hour can run them unchanged.

### R7. Methodology written down

`docs/METHODOLOGY.md`: machines and conditions, clocks (the 100 ns `Instant` step and the TSC), the harness contract, the statistics and the reading rule (p < 0.01 and ratio CI excluding 1.0), the three Windows runs, counters, every other benchmark's command, and a noise budget (session drift 1.3–1.7x; within-session 1.4x; scheduler tail mean/median up to 3x; a swapping VM on the host widens every CI).

**Keywords:** clock resolution, noise budget, reproducibility, benchmark hygiene, measurement bias.

**Google scale.** Performance regressions at scale are caught by comparing numbers taken on different days on different machines, which is only possible if the method that produced them is written down with the same care as the code. A noise budget tells a reviewer which differences in a report are worth a comment and which are weather. The document exists so that the next number anyone adds to this repository has a column for "how was this run".

## 4. What changed, by file

| Path | Change |
| --- | --- |
| `src/wal.rs` | `Scan`/`scan_records`; `open` truncates past the valid prefix (resurrection fix); `write_pos` public |
| `src/snapshot.rs` | checksum covers all fields; new test |
| `src/recovery.rs` | `recover_with_wal_size` |
| `src/lib.rs` | `wal`, `snapshot`, `recovery` public |
| `src/ring.rs` | `Shared<T>` (ManuallyDrop Arc); `ring_allow_unsound`; standalone test payload |
| `tests/crash_recovery.rs` | New: fault-injection harness (5 tests) |
| `verify/ring-standalone/` | New: zero-dependency crate for Miri/TSan |
| `fuzz/` | New: cargo-fuzz crate, 3 targets |
| `examples/ring_throughput.rs`, `cpp/bench_ring.cpp` | `--pin`, `--raw`, `--workload` |
| `scripts/bench_compare.py`, `scripts/perf_counters.sh`, `scripts/asm_loop.sh`, `scripts/wsl_queue.sh` | New |
| `scripts/asm_probe.rs`, `cpp/asm_probe.cpp` | loop probes |
| `docs/asm/*latency*` | New: loop bodies; push/pop files regenerated (label numbers only) |
| `docs/bench/` | New: raw samples and tables of every statistics run |
| `docs/METHODOLOGY.md` | New |
| `docs/MEMORY_ORDERING.md` §3, §5; `docs/VERIFICATION.md` §3–6; `docs/METRICS.md`; `docs/PHASES.md`; `README.md` | Updated |
| `.github/workflows/verify.yml` | Mirrors the queue; standalone crate for Miri/TSan; Rust TSan negative control; fuzz smoke |
| `Cargo.toml`, `.gitignore` | `ring_allow_unsound` cfg; build outputs |

## 5. Keywords earned (general)

fault injection and crash consistency · write-ahead logging and idempotent recovery · LLVM IR reading, escape analysis and register promotion · benchmark statistics (bootstrap CIs, Mann-Whitney, effect size) · CPU affinity and noise budgeting · ThreadSanitizer / ASan / UBSan with negative controls · Miri and strict provenance · coverage-guided fuzzing

## 6. What remains unverified

Status of the Linux queue (`~/ferrox-build/logs/queue.log` in WSL) at the time this report was finalized is recorded in the table below. The last queue run started at 01:29 (WSL clock) and was still inside its first step (compiling the loom test) when the VM stopped responding (`Wsl/Service/HCS_E_CONNECTION_TIMEOUT`); earlier attempts died the same way. "Not completed" means the step never finished on the memory-starved VM; it does not mean it failed.

| step | status |
| --- | --- |
| C++ TSan / ASan+UBSan / negative control | completed, pass / pass / caught (R4) |
| loom on Linux + negative control | not completed (passes on Windows, VERIFICATION.md §1) |
| Miri on the ring | not completed |
| Rust TSan + negative control | not completed |
| `perf stat` counters | not completed |
| pinned Linux variance study (Rust, g++, clang++, unchecked) | not completed (Windows study completed, R3) |
| fuzzing, 3 × 600 s + coverage | not completed |
| full test suite incl. crash harness on Linux | not completed (passes on Windows) |

Also not claimed: power-loss durability of the WAL/snapshot (no `fsync`/`msync` ordering; the harness tests process crashes only); any number from a quiet machine (every 2026-10-11 run shared the host with a swapping VM; `docs/METHODOLOGY.md §8` says how much that is worth); GitHub Actions has never executed `verify.yml` (the branch has not been pushed).
