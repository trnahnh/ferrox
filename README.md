# Ferrox

A low-latency order matching engine in Rust. **500ns P99 latency, 4.7M orders/sec**, zero heap allocation on the hot path.

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

| Benchmark | SPSC Ring | std::sync::mpsc | Speedup |
| --- | --- | --- | --- |
| u64 throughput (1M) | 1.85 ms | 16.35 ms | **8.8x** |
| Order throughput (1M) | 5.51 ms | 20.03 ms | **3.6x** |
| Push+pop latency | 2.11 ns | — | — |

540M ops/sec for raw values. 182M ops/sec for 48-byte Order structs.

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

| Percentile | Latency |
| --- | --- |
| P50 | 100 ns |
| P90 | 200 ns |
| P99 | 500 ns |
| P99.9 | 800 ns |

4.7M orders/sec sustained throughput. Ring pop → match → encode execution report — the full hot path measured per order.

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

137 tests (with `--features metrics`). ~0.25s test time. 6 `unsafe` blocks total (4 in ring buffer, 2 in WAL mmap), each with documented safety invariants.

## Quick Start

```bash
cargo build --release     # build
cargo test                # 134 tests
cargo bench               # criterion benchmarks (matching, ring buffer, WAL, snapshots, e2e)
cargo run --release --example loadgen --features metrics  # latency histogram
```

## Documentation

- [System Design](docs/SYSTEM_DESIGN.md) — architecture, failure analysis, hardware considerations, deployment strategy
- [Development Phases](docs/PHASES.md) — deliverables, stack choices, and rationale per phase
- [Performance Metrics](docs/METRICS.md) — before/after benchmark numbers for every optimization

## Contact

**Anh Tran** — [anhdtran.forwork@gmail.com](mailto:anhdtran.forwork@gmail.com)

## License

MIT
