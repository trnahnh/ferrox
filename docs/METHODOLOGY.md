# Benchmark Methodology

How every number in this repository is produced, what moves it, and how much
noise to expect before believing a difference. Written 2026-10-11 after two
rounds of re-measurement found that several published figures could not be
reproduced ([METRICS.md, "Re-measurement"](METRICS.md#re-measurement-2026-10-10)).

## 1. Machines and conditions

| | development laptop | WSL2 guest on the same laptop |
| --- | --- | --- |
| CPU | AMD Ryzen 7 7735HS, 8 cores / 16 threads (Zen 3+), SMT siblings are (0,1), (2,3), … | 16 vCPUs mapped onto the same cores; `cpu2` and `cpu4` report siblings `2-3` and `4-5`, so they are distinct physical cores |
| OS | Windows 11, power plan "Turbo" | Ubuntu 24.04, kernel 5.15.167.4-microsoft-standard-WSL2 |
| Memory | 32 GiB; during the 2026-10-11 session a 6.5 GiB `llama-server` and Docker Desktop were also running | VM capped at 3.9 GiB and *shared with the `docker-desktop` distro*, whose containers held ~3.5 GiB; everything built there swapped (load average 100–190) |
| Rust | 1.93.1 (LLVM 21.1.8) | stable 1.99.0, nightly 1.101.0 (2026-10-10) |
| C++ | g++ 14.2.0 (MSYS2 UCRT64) | g++ 13.3.0, clang++ 18.1.3 |

Hardware counters: Windows exposes none to user code without a driver; the
WSL2 kernel reports `Performance Events: Fam17h+ core perfctr, AMD PMU driver`
and `perf_event_paranoid = 2`, so `perf stat` on one's own process is allowed.
What the hypervisor actually virtualizes is recorded in section 6, counter by
counter, including `<not supported>`.

## 2. Clocks

- `std::time::Instant` on this Windows machine advances in **100 ns** steps
  (QueryPerformanceCounter at 10 MHz; measured over 5M consecutive reads, every
  non-zero delta was a multiple of 100 ns). Any per-event latency below a few
  microseconds measured with it is quantized; percentiles come out as multiples
  of 100 ns. This is what produced the Phase 7 "P99 = 500 ns".
- `examples/loadgen.rs` therefore times each order with the x86-64 TSC
  (`rdtsc`, 0.3 ns step at 3.19 GHz, calibrated against `Instant` for 200 ms at
  start-up) and prints the observed clock step next to the percentiles.
  `--instant` reproduces the old method so the two can be compared.
- The ring harnesses time a whole run of 1M items with `Instant` /
  `steady_clock`; at 2–20 ms per run the 100 ns step is irrelevant (< 0.01%).
- On Linux, `Instant` is `clock_gettime(CLOCK_MONOTONIC)` with ~20 ns
  resolution; no correction is needed there.

## 3. The ring harnesses (Rust vs C++, knob studies)

`examples/ring_throughput.rs` and `cpp/bench_ring.cpp` implement one
methodology so the two languages and the compile-time knobs can be compared
like for like:

- three workloads: `u64` (1M 8-byte items across two threads), `order` (1M
  48-byte items across two threads), `latency` (1M push+pop pairs on one
  thread, reported as ns per pair);
- producer on a spawned thread, consumer on the calling thread, spin-wait with
  `pause`; thread spawn and join are *inside* the timed region (as in the
  original criterion benchmark), which adds roughly 50–150 µs of OS noise per
  run and is the main reason cross-thread runs have a long upper tail;
- `--pin P,C` pins the producer to logical CPU P and the consumer to C
  (`sched_setaffinity` / `SetThreadAffinityMask`), `--raw` prints every trial,
  `--workload` selects one workload;
- the knobs (`ring_seqcst`, `ring_no_padding`, `ring_no_cursor_cache`,
  `ring_unchecked_index` in Rust; `-DFERROX_*` in C++) change one thing each
  and are never on by default ([ring.rs](../src/ring.rs) module docs).

`benches/ring_bench.rs` (criterion) measures the same ring against
`std::sync::mpsc`; it is kept for the historical tables but is not used for
language or knob comparisons because the C++ side has no criterion.

## 4. Statistics: `scripts/bench_compare.py`

Two binaries measured one after the other are not comparable on this machine:
the first run of a session is routinely 1.3–1.7x slower or faster than the
same binary an hour later. The driver removes that bias by **interleaving**:
every round runs every variant once (4 trials each), alternating the order
between rounds (ABBA), for 10 rounds, so slow drift hits all variants equally
and n = 40 per variant and workload. It then reports, in pure Python so the
environment adds no dependency:

| statistic | why |
| --- | --- |
| median and a 95% bootstrap CI (10,000 resamples, fixed seed) | the distributions are skewed with a long upper tail (scheduler), so the mean and standard deviation mislead |
| ratio of medians vs the first variant, with a bootstrap CI | the quantity people actually quote; the CI says whether 1.0 is plausible |
| Mann-Whitney U, two-sided, tie-corrected normal approximation | distribution-free test that one variant tends to be faster; at n = 40 the approximation is accurate |
| Cliff's delta | effect size on a −1..1 scale; by convention \|d\| < 0.147 is negligible, < 0.33 small, < 0.474 medium, else large |

Reading rule used throughout the docs: a difference is reported as real only if
p < 0.01 **and** the ratio CI excludes 1.0; otherwise it is "a tie within
noise", whatever the medians say.

Raw samples and statistics for every run quoted below are in
[`docs/bench/`](bench/) as JSON (rerunnable: the JSON records the command,
pin, counts and all 40 samples).

## 5. What the pinned, interleaved comparison shows (Windows, 2026-10-11)

Variants: `rust` (this branch), `rust_before_fix` (same harness, ring from
before the register-promotion fix of [MEMORY_ORDERING.md §5](MEMORY_ORDERING.md#5-the-single-thread-gap-explained-and-closed-2026-10-11)),
`rust_unchecked` (`--cfg ring_unchecked_index`), `cpp_gcc` (g++ 14.2, `-O3`).
Three runs, all with the WSL2 VM swapping on the same host (see §1); run 1 was
the quietest.

| run / workload | rust median | compared variant | median | ratio vs rust [95% CI] | p | Cliff's δ | verdict |
| --- | --- | --- | --- | --- | --- | --- | --- |
| run 1 pinned 2,4 / u64 | 5.458 ms | cpp_gcc | 5.616 ms | 1.029 [0.960, 1.104] | 0.31 | +0.13 | tie |
| run 1 pinned / u64 | | rust_before_fix | 5.703 ms | 1.045 [0.949, 1.120] | 0.90 | +0.02 | tie (fix is irrelevant cross-thread on u64) |
| run 1 pinned / order48 | 12.553 ms | cpp_gcc | 14.080 ms | 1.122 [1.074, 1.189] | 8.5e-05 | +0.51 | **Rust faster** |
| run 1 pinned / order48 | | rust_before_fix | 14.494 ms | 1.155 [1.102, 1.214] | 2.3e-06 | +0.62 | fix helps cross-thread too |
| run 1 pinned / latency | 1.610 ns | cpp_gcc | 1.196 ns | 0.743 [0.691, 0.781] | 3.9e-10 | −0.81 | **C++ faster** (1.35x) |
| run 1 pinned / latency | | rust_before_fix | 2.752 ns | 1.710 [1.552, 1.916] | 1.4e-14 | +1.00 | fix: 1.7x |
| run 2 pinned 2,4 / u64 | 7.111 ms | cpp_gcc | 7.760 ms | 1.091 [0.940, 1.346] | 0.95 | +0.01 | tie |
| run 2 pinned / order48 | 14.142 ms | cpp_gcc | 16.609 ms | 1.174 [1.043, 1.323] | 0.0014 | +0.42 | Rust faster |
| run 2 pinned / latency | 2.679 ns | cpp_gcc | 1.577 ns | 0.589 [0.491, 0.692] | 5.7e-09 | −0.76 | C++ faster |
| run 2 pinned / latency | | rust_unchecked | 1.909 ns | 0.712 [0.588, 0.949] | 0.0012 | −0.42 | bounds check is most of the rest |
| run 2 unpinned / u64 | 4.127 ms | cpp_gcc | 4.943 ms | 1.198 [1.059, 1.377] | 0.0004 | +0.46 | Rust faster (not seen pinned) |
| run 2 unpinned / order48 | 11.682 ms | cpp_gcc | 12.920 ms | 1.106 [1.039, 1.180] | 0.0031 | +0.39 | Rust faster |
| run 2 unpinned / latency | 2.075 ns | cpp_gcc | 1.365 ns | 0.658 [0.528, 0.771] | 9.0e-07 | −0.64 | C++ faster |
| run 2 unpinned / latency | | rust_unchecked | 1.942 ns | 0.936 [0.702, 1.091] | 0.045 | −0.26 | not significant in this run |

Conclusions that survive all three runs:

1. **Cross-thread, 48-byte payloads: Rust is 10–17% faster than the g++ build,
   p ≤ 0.003 every time.** This is new; round 1 called it a tie because two
   unpinned, un-interleaved runs could not resolve it.
2. **Cross-thread, 8-byte payloads: a tie** in both pinned runs; the one
   unpinned run that favored Rust is not corroborated.
3. **Single-thread push+pop: C++ is 1.35–1.7x faster than the fixed Rust**,
   down from 2.3–3.5x before the fix; removing the bounds check removes most
   of what is left (run 2 pinned: 1.909 vs 1.577 ns).
4. The register-promotion fix is real in every run (p ≤ 4.6e-05 on latency,
   Cliff's δ 0.53–1.00).

What pinning did and did not do here: with the hypervisor's `vmmem` process
stealing time on the same physical cores, pinning did **not** shrink the
confidence intervals (run 2 pinned has the widest CIs of the three); its value
is removing cross-core migration as a variable, not removing the neighbor.
Run 1 is the cleanest and is the one quoted elsewhere; the others are kept
because they are what the machine actually did.

## 6. Hardware counters (`perf stat`, WSL2)

`scripts/perf_counters.sh` builds the four ring variants and runs the u64
workload under `perf stat -r 5` with cycles, instructions, cache references and
misses, LLC loads and misses, branches and branch misses, context switches and
migrations. Results: see ROUND2_REPORT.md §3 (this section is filled in from
`docs/perf/*.csv` once the WSL run completes; if a counter prints
`<not supported>` that is reported as such).

## 7. Other benchmarks

| benchmark | command | what to watch |
| --- | --- | --- |
| matching engine microbenchmarks | `cargo bench --bench matching_bench` | criterion, unpinned; Phase 1→2→3 tables need `git checkout f019ec3 / 0e3da15 / 5d28e69` for the old implementations |
| WAL, snapshot, recovery | `cargo bench --bench wal_bench` | the WAL is `mmap`-backed: `append` cost excludes page-cache writeback |
| end-to-end | `cargo bench --bench e2e_bench` | single-threaded; still drifted 1.2–1.7x between sessions in round 1 |
| load generator | `cargo run --release --example loadgen --features metrics` | TSC-timed per order; prints clock step, fill/rest split, and (with `alloc-track`) allocations; two clock reads per order cost about 10% throughput, so `cargo run --release --example loadgen` without `metrics` is the throughput number |
| crash consistency | `cargo test --release --test crash_recovery -- --nocapture` | not a benchmark; ~55–75 s on Windows (NTFS temp-dir churn), seconds on Linux |

## 8. Noise budget: what to expect before calling anything a result

| source | size | mitigation |
| --- | --- | --- |
| session-to-session drift (power state, background load) | 1.3–1.7x on identical binaries, seen in both rounds | never compare across sessions; interleave variants within one |
| run-to-run within a session, unpinned | up to 1.4x on the same binary | n ≥ 40 per variant, medians, bootstrap CIs |
| thread spawn + scheduler tail inside the timed region | mean/median gap of 1.1–3x on cross-thread runs (run 2 pinned u64: mean 21 ms, median 7.1 ms) | report medians and minima, never means |
| a hypervisor VM swapping on the same host | widened every CI in run 2 vs run 1 | note it; rerun when quiet |
| `Instant` quantization (Windows) | 100 ns | TSC for per-event timing; irrelevant above ~10 µs |
| pinning | removes migration; does not remove neighbors on SMT siblings or the hypervisor | pin to distinct physical cores (2 and 4 here) |

A difference smaller than 10% between two ring variants is not resolvable on
this machine without the interleaved driver, and even with it a 10% difference
needs n = 40 to reach p < 0.01. Anything quoted in these docs below that
threshold is marked "within noise".

## Reproduce

```bash
# build the three Windows/Linux variants
cargo build --release --example ring_throughput
RUSTFLAGS="--cfg ring_unchecked_index" cargo build --release --example ring_throughput --target-dir target/unchecked
g++ -std=c++20 -O3 -pthread cpp/bench_ring.cpp -o cpp/bench_ring

# pinned, interleaved, 10 rounds x 4 trials (needs python3; on Windows run it from WSL, the .exe paths work through interop)
python3 scripts/bench_compare.py --pin 2,4 --rounds 10 --iters 4 --json out.json \
    rust=target/release/examples/ring_throughput \
    rust_unchecked=target/unchecked/release/examples/ring_throughput \
    cpp_gcc=cpp/bench_ring

# hardware counters (Linux)
bash scripts/perf_counters.sh 2,4 5
```
