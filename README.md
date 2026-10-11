# Ferrox

A low-latency order matching engine in Rust. **Sub-microsecond per-order latency (P99 0.83–1.06 µs, TSC-timed), 4.36–4.41M orders/sec**, zero heap allocation on the hot path for orders and fills. Every number in this README comes from a command you can rerun; the ring buffer is model-checked under the C11 memory model and ported to C++ for a like-for-like comparison.

## The Problem

At the core of every exchange sits a matching engine — the component that takes buy and sell orders and pairs them together. The difference between a good matching engine and a great one is measured in microseconds, and those microseconds translate directly to money.

Most implementations get this wrong in subtle ways:

- **Floating-point prices** introduce IEEE 754 rounding errors. A price of `0.1 + 0.2 != 0.3` is unacceptable when real money is on the line.
- **Heap allocation on every order** means the OS allocator sits in the critical path. `malloc` contention and memory fragmentation create unpredictable latency spikes.
- **Lock-based concurrency** forces threads to wait on each other. Mutex contention, cache line bouncing, and context switches destroy throughput under load.
- **Naive data structures** like `HashMap` + `VecDeque` work for correctness but leave performance on the table — O(n) scans to find the best price, pointer chasing across scattered memory, no cache locality.

Ferrox exists to solve each of these problems, one phase at a time, with hard benchmark numbers proving every optimization.

## The Approach

### Phase 1 — Get It Right First

Started with the simplest correct implementation: `HashMap<i64, VecDeque<Order>>` for price levels, linear scan for best price. Prices stored as `i64` fixed-point ticks from day one (`$150.05` = `15005` at tick size `0.01`) — no floating point anywhere on the trade path.

Built 37 tests including property-based testing with `proptest` to verify invariants: quantity conservation, no crossed book after matching, no self-trade fills, all fill quantities positive.

### Phase 2 — Eliminate the Allocator

Replaced `VecDeque` with an arena-based object pool and intrusive doubly linked lists. Every `OrderNode` is 64 bytes (`#[repr(C, align(64))]`) — exactly one cache line. A pre-allocated arena with a free list gives O(1) alloc/dealloc with zero syscalls. Cancel went from O(n) linear scan to O(1) unlink.

| Benchmark | Before | After | Change |
| --- | --- | --- | --- |
| cancel_middle_of_1k | 2.16 us | 0.91 us | **-58%** |
| mixed_workload_10k | 1.19 ms | 0.69 ms | **-42%** |

### Phase 3 — Stop Scanning for Best Price

Replaced `HashMap` with `BTreeMap` for price levels. Best-price recomputation after a level empties went from O(n) linear scan to O(log n) tree lookup.

| Benchmark | Before | After | Change |
| --- | --- | --- | --- |
| match/multi_level_sweep | 45.14 us | 14.73 us | **-67%** |
| cancel_best_level/1000 | 696.94 us | 124.07 us | **-82%** |

### Phase 4 — Lock-Free Cross-Thread Communication

Built a Disruptor-style SPSC ring buffer for the ingestion-to-matching pipeline. Free-running `AtomicUsize` cursors with bitmask indexing, `CachePadded` atomics to prevent false sharing, and local cursor caching to minimize cross-core cache bouncing. Acquire/Release ordering compiles to plain MOV instructions on x86 — zero barrier overhead.

| Benchmark (2026-10-10 run) | SPSC Ring | std::sync::mpsc (blocking recv) | std::sync::mpsc (try_recv spin) | Speedup vs best mpsc |
| --- | --- | --- | --- | --- |
| u64 throughput (1M) | 3.06 ms | 11.80 ms | 9.73 ms | **3.2x** |
| Order (48B) throughput (1M) | 12.79 ms | 14.92 ms | 27.36 ms | **1.17x** |
| Push+pop latency (1 thread) | 2.37 ns | — | — | — |

327M ops/sec for raw values, 78M ops/sec for 48-byte Order structs, on an AMD Ryzen 7 7735HS laptop. An earlier session recorded 1.85 ms vs 16.35 ms (8.8x) and 5.51 ms vs 20.03 ms (3.6x); those gaps have not been reproduced since. This benchmark includes thread creation and runs unpinned, and the same binary varies 1.5x between sessions, so treat the ratio as "several times faster for small payloads, roughly even for 48-byte payloads". The "plain MOV" claim above is verified from the generated assembly in [docs/MEMORY_ORDERING.md](docs/MEMORY_ORDERING.md).

### Phase 5 — Binary Protocol + Networking

Built a zero-dependency binary wire protocol: `NewOrder` (40B), `CancelOrder` (16B), `ExecutionReport` (48B) — little-endian, fixed-size, `from_le_bytes`/`to_le_bytes`. No JSON, no Protobuf on the hot path.

Wired up the full 2-thread architecture: TCP ingestion reads orders, pushes them through the SPSC ring buffer to the matching thread, which broadcasts `ExecutionReport` messages over UDP multicast with monotonic sequence numbers for gap detection.

### Phase 6 — Crash Recovery

Added write-ahead logging and periodic snapshots for deterministic crash recovery. Every command is persisted to an mmap-backed WAL before matching. Snapshots are taken every N commands (configurable) to bound recovery time.

| Benchmark | Time | Notes |
| --- | --- | --- |
| WAL append (encode + CRC32) | 56 ns/op | Hot-path overhead per order |
| Snapshot capture + serialize (10K orders) | 126 us | Amortized ~12.6 ns/order at default interval |
| Full recovery (snapshot + 10K WAL replay) | 1.4 ms | Worst case |

WAL uses the existing `protocol.rs` codec — no duplicate serialization. Pre-allocated encode buffer means zero allocation on the hot path. Recovery is deterministic: same WAL replayed twice produces bit-exact book state.

### Phase 7 — Prove It

Eliminated the last hot-path allocation: `AddOrderResult.fills` changed from `Vec<Fill>` to `&[Fill]` borrowed from the engine's pre-allocated buffer. Added HdrHistogram latency recording, a tracking allocator, end-to-end criterion benchmarks, and a load generator.

| Percentile (2026-10-10, TSC clock) | Run 1 | Run 2 | Run 3 |
| --- | --- | --- | --- |
| P50 | 130 ns | 210 ns | 190 ns |
| P90 | 321 ns | 481 ns | 431 ns |
| P99 | 832 ns | 1062 ns | 982 ns |
| P99.9 | 1404 ns | 1584 ns | 1684 ns |
| Mean | 171 ns | 257 ns | 250 ns |

Ring pop → match → encode execution report, measured per order over 990,000 orders (about 63% fill, 37% rest in the book). Throughput is 3.21–4.01M orders/sec with two clock reads per order for the histogram and 4.36–4.41M orders/sec without timing.

The earlier published figures (P50/P90/P99 = 100/200/500 ns, 4.7M/s) were taken with `std::time::Instant`, which on Windows advances in 100 ns steps, so each percentile was quantized to that grid. The load generator now uses the x86-64 timestamp counter (0.3 ns step) and prints the clock step it observed; `--instant` reproduces the old method. Details in [docs/VERIFICATION.md §5](docs/VERIFICATION.md#5-measurement-checks-that-came-out-of-the-audit).

### Phase 8 — Verify It (Memory Model, Differential Tests, C++ Port)

Benchmarks say the ring is fast; this phase asks whether it is *right*, and what each design decision is worth.

- **Model checking.** The ring compiles against [loom](https://docs.rs/loom) (`--cfg loom`), which explores every interleaving the C11 memory model allows and checks every slot access for a data race. FIFO across wraparound and backpressure, and exactly-once drop, pass. Downgrading the cursors to `Relaxed` (`--cfg ring_relaxed_ordering`) is caught immediately: `Causality violation: Concurrent read and write accesses.`
- **Differential testing.** Random add/cancel sequences run through the engine and a naive flat-`Vec` reference book; status, fills, cancels, best prices and full book contents must agree after every operation, and replaying the same input must produce the same transcript. No disagreement in 512 cases per property.
- **C++ port.** `cpp/spsc_ring.hpp` is a line-for-line C++20 port with the same compile-time knobs. Cross-thread throughput is a tie within noise (medians favor Rust by 10–20%, minimums favor C++ by 5–10%); single-threaded push+pop is 1.5–3.5x faster in C++, partly the bounds check safe Rust cannot elide (10–25% of the loop), the rest unexplained.
- **What the design decisions cost**, same session, one knob at a time: SeqCst instead of acquire/release is 5.4x (Rust) / 6.0x (C++) slower on u64 transfer; putting both cursors on one cache line is 2x slower; dropping the cached remote cursor is 1.4–1.7x slower. Both compilers emit plain `mov` for the release stores and `xchg` under SeqCst.

Full numbers and reproduction: [docs/MEMORY_ORDERING.md](docs/MEMORY_ORDERING.md), [docs/VERIFICATION.md](docs/VERIFICATION.md). Miri and ThreadSanitizer/ASan/UBSan need Linux and run in [CI](.github/workflows/verify.yml), which has not been executed yet.

### Cloud Deployment — The Decoupled Subscriber

The matching core never leaves bare metal (pinned CPUs, in-process, no cloud — see [System Design §11](docs/SYSTEM_DESIGN.md#11-deployment-strategy)). The one piece that *is* decoupled — the UDP subscriber that does gap detection on the multicast execution-report feed — was actually deployed: containerized, pushed to ECR, and run on AWS Fargate behind a Network Load Balancer with real Terraform, in a real AWS account.

Lambda doesn't fit a long-lived UDP listener (no event source delivers raw UDP, no persistent socket across invocations) — that's not a close call. Fargate does, directly. The real complication was networking: UDP multicast doesn't traverse a VPC, so the deployment uses a unicast relay at the network edge instead of trying to bridge multicast into AWS.

Measured, not estimated: **~$0.041/hr** to run (70% of that is the NLB, not the compute — worth knowing before reaching for a load balancer), **45–70s** cold start dominated by NLB health-check convergence, and an honest dead end on cross-clock latency measurement from this environment (documented rather than papered over). Full writeup: [docs/CLOUD_DEPLOYMENT.md](docs/CLOUD_DEPLOYMENT.md). Architecture rationale: [System Design §13](docs/SYSTEM_DESIGN.md).

## Current State

| Phase | Focus | Status |
| --- | --- | --- |
| 1 | Domain model, matching logic, property-based tests | Done |
| 2 | Arena object pool, intrusive linked lists, cache-line alignment | Done |
| 3 | BTreeMap sorted price levels, O(log n) best-price recomputation | Done |
| 4 | Lock-free SPSC ring buffer (Disruptor pattern) | Done |
| 5 | Binary protocol, TCP/UDP networking, 2-thread pipeline | Done |
| 6 | WAL persistence, snapshots, deterministic crash recovery | Done |
| 7 | End-to-end benchmarking suite, HdrHistogram, observability | Done |
| 8 | loom model checking, differential property tests, C++ port, memory-ordering and cache analysis | Done (sanitizer CI not yet run) |

141 tests with `--features metrics` (137 unit/property + 4 differential), plus 4 loom model checks under `--cfg loom` and 6 C++ tests. `unsafe` inventory: 6 blocks + 3 `unsafe impl` in the ring buffer (3 primitive slot operations and their 3 call sites), 2 blocks in the WAL mmap, 2 blocks + 1 `unsafe impl` in the tracking allocator; every one has a documented invariant. See [docs/VERIFICATION.md §3](docs/VERIFICATION.md#3-unsafe-inventory).

## Quick Start

```bash
cargo build --release     # build
cargo test                # 138 tests (141 with --features metrics)
cargo bench               # criterion benchmarks (matching, ring buffer, WAL, snapshots, e2e)
cargo run --release --example loadgen --features metrics  # per-order latency histogram (TSC clock)
cargo run --release --example loadgen                     # throughput only

RUSTFLAGS="--cfg loom" cargo test --release --test loom_ring       # model-check the ring
cargo run --release --example ring_throughput -- 30                  # Rust ring harness
g++ -std=c++20 -O3 -pthread cpp/bench_ring.cpp -o cpp/bench_ring && cpp/bench_ring 30   # C++ port, same harness
bash scripts/asm_ordering.sh                                         # acquire/release vs SeqCst assembly
```

## Documentation

- [System Design](docs/SYSTEM_DESIGN.md) — architecture, failure analysis, hardware considerations, deployment strategy
- [Development Phases](docs/PHASES.md) — deliverables, stack choices, and rationale per phase
- [Performance Metrics](docs/METRICS.md) — before/after benchmark numbers for every optimization, and the 2026-10-10 re-measurement
- [Verification](docs/VERIFICATION.md) — loom model checking, differential tests, `unsafe` inventory, sanitizer CI, measurement corrections
- [Memory Ordering and Cache Analysis](docs/MEMORY_ORDERING.md) — generated assembly, cost of SeqCst / false sharing / cursor caching / bounds checks, Rust vs C++
- [Cloud Deployment](docs/CLOUD_DEPLOYMENT.md) — the decoupled subscriber on Fargate: measured cost, cold start, and what didn't work cleanly

## Contact

**Anh Tran** — [anhdtran.forwork@gmail.com](mailto:anhdtran.forwork@gmail.com)

## License

MIT
