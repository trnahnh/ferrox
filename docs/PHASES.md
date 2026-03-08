# Development Phases

## Phase 1: Domain Model + Matching Logic

**Objective**: Build a correct, fully-tested order book with price-time priority matching. Correctness first, no optimization.

**Deliverables**:

- `Order` struct: `id` (u64), `side` (enum Bid/Ask), `price` (i64 fixed-point ticks), `quantity` (u64), `timestamp` (u64 nanoseconds)
- `OrderBook` with `HashMap<i64, VecDeque<Order>>` for price levels
- Price-time priority matching: best price first, FIFO within price level
- Operations: `add_order`, `cancel_order`, `match_order` with partial fill support
- Full unit test suite: add, cancel, partial fill, self-trade prevention, empty book edge cases

**Stack**:

| Component | Choice | Rationale |
| --------- | ------ | --------- |
| Language | Rust (2021 edition) | No GC, zero-cost abstractions |
| Price Representation | i64 (fixed-point ticks) | No floating-point errors |
| Price Level Storage | `std::collections::HashMap` | O(1) price lookup, naive first pass |
| Order Queue | `std::collections::VecDeque` | FIFO, simple, correct baseline |
| Testing | `cargo test` + `proptest` | Property-based testing for edge cases |

**Constraint**: Prices stored as integer ticks (e.g., $150.05 = 15005 at tick_size=0.01). No `f64` anywhere on the trade path.

---

## Phase 2: Performance Data Structures

**Objective**: Replace naive collections with cache-friendly, zero-allocation data structures. Prove systems-level thinking with before/after benchmark numbers.

**Deliverables**:

- Intrusive doubly linked list per price level (replaces `VecDeque`)
- Arena-based object pool: pre-allocate 1M `Order` slots at startup, zero `malloc` on hot path
- Cache line padding (`#[repr(align(64))]`) on shared structures to prevent false sharing
- Criterion benchmark suite: naive (Phase 1) vs optimized (Phase 2)
- Memory profiling confirming zero heap allocation during matching

**Stack**:

| Component | Choice | Rationale |
| --------- | ------ | --------- |
| Linked List | Custom intrusive list (`unsafe`) | O(1) insert/remove, no pointer chasing |
| Object Pool | Custom `Arena<Order>` | Pre-allocated slab, index-based access |
| Cache Alignment | `#[repr(align(64))]` | Prevent false sharing between threads |
| Benchmarking | `criterion` (v0.5+) | Statistical benchmarks with regression detection |
| Memory Profiling | DHAT / heaptrack | Verify zero-allocation hot path |

**Key Detail**: The intrusive linked list requires `unsafe` Rust. Each `Order` node contains prev/next indices into the arena rather than `Box` pointers. This eliminates heap allocation and improves cache locality. Safety invariants documented in code comments.

---

## Phase 3: Sorted Price Levels

**Objective**: Replace HashMap with BTreeMap for price levels, improving best-price recomputation from O(n) to O(log n).

**Deliverables**:

- `BTreeMap<i64, PriceLevel>` for bid/ask sides (sorted by price)
- Best-price recomputation via `keys().next_back()` (bids) / `keys().next()` (asks)
- Benchmark: cancel and matching improvements vs Phase 2

**Status**: Complete.

---

## Phase 4: Lock-Free Ring Buffer (Disruptor Pattern)

**Objective**: Implement single-producer single-consumer communication between the ingestion thread and the matching engine without locks or kernel involvement.

**Deliverables**:

- SPSC ring buffer using `AtomicUsize` for head/tail with `Acquire`/`Release` memory ordering
- Pre-allocated fixed-size buffer (power-of-two capacity for fast modulo via bitmask)
- Custom `CachePadded<T>` wrapper (`#[repr(align(64))]`, zero dependencies)
- Local cursor caching to reduce cross-core atomic loads
- Benchmark: ring buffer throughput/latency vs `std::sync::mpsc::channel`

**Stack**:

| Component | Choice | Rationale |
| --------- | ------ | --------- |
| Atomics | `std::sync::atomic::AtomicUsize` | Lock-free synchronization |
| Memory Ordering | `Ordering::Acquire` / `Release` | Minimal barrier, correct visibility |
| Buffer Layout | Power-of-2 array + bitmask | `index & (capacity - 1)` replaces modulo |
| Padding | Custom `CachePadded` (`#[repr(align(64))]`) | Separate head/tail to different cache lines, zero deps |
| Comparison Baseline | `std::sync::mpsc` | Show improvement over stdlib channel |

**Status**: Complete.

---

## Phase 5: Binary Protocol + UDP Networking

**Objective**: Add a binary wire protocol and TCP/UDP networking, wiring up the 2-thread architecture: TCP ingestion → ring buffer → matching → UDP multicast.

**Deliverables**:

- Wire protocol: `NewOrder` (40B), `CancelOrder` (16B), `ExecutionReport` (48B) — little-endian, fixed-size
- `EngineCommand` enum as ring buffer payload between ingestion and matching threads
- TCP ingestion: read-exact framing (first byte = msg_type → known size), timestamp assignment
- UDP multicast publisher: matching thread broadcasts `ExecutionReport` with monotonic `seq_num`
- Subscriber example (`examples/subscriber.rs`) with sequence gap detection
- 2-thread gateway: `GatewayConfig` for listen addr, multicast addr, ring/arena capacity

**Stack**:

| Component | Choice | Rationale |
| --------- | ------ | --------- |
| Binary Encoding | Custom flat codec (`from_le_bytes`/`to_le_bytes`) | Zero-copy, no deps, no schema overhead |
| TCP Ingestion | `std::net::TcpListener` | Single-client, blocking read_exact |
| UDP Multicast | `std::net::UdpSocket` | Multicast TTL=1, send_to() |
| Gap Detection | Monotonic `seq_num` on ExecutionReport | Subscriber tracks expected seq |
| Ring Buffer | Phase 4 SPSC ring | Lock-free EngineCommand transfer |

**Constraint**: No JSON, no Protobuf on the hot path. Messages are fixed-size structs encoded via `to_le_bytes`/`from_le_bytes`. Zero new dependencies.

**Status**: Complete.

---

## Phase 6: Persistence + Deterministic Replay

**Objective**: Implement crash recovery through event sourcing. Every order is logged before processing, and the full book state can be recovered by replaying the log.

**Deliverables**:

- Write-ahead log (WAL) using memory-mapped files for sequential writes
- Every inbound order serialized to WAL before matching engine processes it
- Deterministic replay: feed WAL back through engine, assert bit-exact book state
- Periodic snapshots every N orders (configurable) to limit replay time
- Crash recovery test: truncate WAL at random points, verify correct recovery from last snapshot

**Stack**:

| Component | Choice | Rationale |
| --------- | ------ | --------- |
| Memory-Mapped I/O | `memmap2` | OS-managed page cache, sequential write perf |
| WAL Format | Length-prefixed binary records | Simple, fast, no parsing overhead |
| Snapshot Serialization | `bincode` | Fast serde-compatible binary format |
| Integrity Check | CRC32 per record (`crc32fast`) | Detect corruption from partial writes |
| File Management | `std::fs` + `tempfile` (for tests) | Atomic file operations, test isolation |

**Key Invariant**: Determinism. Replaying the same sequence of orders must produce the exact same book state every time. No randomness, no system-time dependencies, no thread-ordering variation in the matching path.

**Status**: Complete.

---

## Phase 7: Benchmarking Suite + Observability

**Objective**: Prove every performance claim with hard data. This is what goes at the top of the README and resume.

**Deliverables**:

- HdrHistogram integration tracking P50, P90, P99, P99.9 tick-to-trade latency
- Load generator pushing 1M+ orders/second through the full pipeline
- Memory usage validation: flat allocation under sustained load (object pool working correctly)
- Latency histogram image exported for README
- System design document finalized with actual benchmark numbers
- Optional: Prometheus metrics endpoint + Grafana dashboard (throughput, latency heatmap, book depth)

**Stack**:

| Component | Choice | Rationale |
| --------- | ------ | --------- |
| Latency Histograms | `hdr_histogram` | Industry-standard latency recording |
| Microbenchmarks | `criterion` (v0.5+) | Statistical rigor, regression detection |
| Load Generation | Custom Rust binary | Control over order distribution patterns |
| Metrics Export | `prometheus` (optional) | Standard metrics pipeline |
| Visualization | Grafana (optional) / `plotters` | Dashboard or static histogram images |
| CPU Profiling | `perf` + `flamegraph` | Identify hot path bottlenecks |

**Kill Metric**: "Sub-50µs P99 latency processing 1M+ orders/sec. Zero heap allocation on the hot path."

**Results**: P99 = 500 ns, throughput = 4.7M orders/sec. Fills allocation eliminated (`&[Fill]` borrow instead of `Vec<Fill>`). BTreeMap node allocation is the only remaining heap operation (level creation/removal).

**Status**: Complete.
