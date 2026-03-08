# Performance Metrics

**Benchmark environment**: Windows 11, Rust 2024 edition, `cargo bench` (criterion 0.8.2, release profile).

---

## Phase 1 → Phase 2: Data Structure Upgrade

**What changed**: `HashMap<i64, VecDeque<Order>>` replaced with arena-based object pool + intrusive doubly linked lists. Cancel complexity: O(n) → O(1).

### Order Cancel

| Benchmark | Phase 1 | Phase 2 | Change |
| --- | --- | --- | --- |
| cancel_all/100 | 6.26 µs | 4.46 µs | -29% |
| cancel_all/1000 | 55.42 µs | 39.79 µs | -28% |
| cancel_all/5000 | 313.87 µs | 208.34 µs | -34% |
| cancel_middle_of_1k | 2.16 µs | 0.91 µs | -58% |

### Matching

| Benchmark | Phase 1 | Phase 2 | Change |
| --- | --- | --- | --- |
| match/full_fill_1k | 96.20 µs | 96.87 µs | ~0% |
| match/multi_level_sweep | 42.07 µs | 50.61 µs | +20% |

### Insert

| Benchmark | Phase 1 | Phase 2 | Change |
| --- | --- | --- | --- |
| insert/non_crossing/100 | 5.15 µs | 5.57 µs | ~0% |
| insert/non_crossing/1000 | 47.91 µs | 49.34 µs | ~0% |
| insert/non_crossing/10000 | 529.28 µs | 537.06 µs | ~0% |

### Mixed Workload

| Benchmark | Phase 1 | Phase 2 | Change |
| --- | --- | --- | --- |
| mixed_workload_10k | 1.19 ms | 0.69 ms | -42% |

### Memory

| Metric | Phase 1 | Phase 2 |
| --- | --- | --- |
| Order storage | Heap-allocated per order (`VecDeque<Order>`) | Pre-allocated arena (1M slots, 64 MB) |
| Order size | 48 bytes (unaligned) | 64 bytes (`#[repr(C, align(64))]`, 1 cache line) |
| Cancel lookup | O(1) index lookup + O(n) linear scan | O(1) index lookup + O(1) unlink |
| Hot-path `malloc` | Every insert/cancel | Zero (arena pre-allocated) |
| `unsafe` blocks | 0 | 0 |

### Test Suite

| Metric | Phase 1 | Phase 2 |
| --- | --- | --- |
| Test count | 37 | 53 |
| Test time | ~0.06s | ~0.13s |

Note: Phase 1 test time measured before arena was introduced. Phase 2 tests use `with_capacity(1024)` to keep arena small (64 KB vs 64 MB default).

---

## Phase 2 → Phase 3: Sorted Price Levels

**What changed**: `HashMap<i64, PriceLevel>` replaced with `BTreeMap<i64, PriceLevel>` for bid/ask sides. Best-price recomputation after level removal: O(n) linear scan → O(log n) BTreeMap min/max.

Note: Phase 2 and Phase 3 numbers below are from the same benchmark session for an apples-to-apples comparison.

### Matching (Phase 2→3)

| Benchmark | Phase 2 | Phase 3 | Change |
| --- | --- | --- | --- |
| match/full_fill_1k | 87.47 µs | 61.49 µs | -30% |
| match/multi_level_sweep | 45.14 µs | 14.73 µs | -67% |

`multi_level_sweep` sweeps 100 ask levels, emptying each and recomputing `best_ask`. With HashMap this was O(n) per level removal; with BTreeMap it's O(log n).

### Order Cancel (Phase 2→3)

| Benchmark | Phase 2 | Phase 3 | Change |
| --- | --- | --- | --- |
| cancel_all/100 | 4.42 µs | 2.05 µs | -54% |
| cancel_all/1000 | 39.56 µs | 20.79 µs | -47% |
| cancel_all/5000 | 206.19 µs | 110.74 µs | -46% |
| cancel_best_level/100 | 35.62 µs | 5.29 µs | -85% |
| cancel_best_level/500 | 254.49 µs | 40.12 µs | -84% |
| cancel_best_level/1000 | 696.94 µs | 124.07 µs | -82% |
| cancel_middle_of_1k | 1.02 µs | 0.85 µs | -17% |

`cancel_best_level` cancels N orders across N distinct prices, emptying a level on every cancel (worst case for best-price recomputation). O(n²) total with HashMap → O(n log n) with BTreeMap.

### Insert (Phase 2→3)

| Benchmark | Phase 2 | Phase 3 | Change |
| --- | --- | --- | --- |
| insert/non_crossing/100 | 8.18 µs | 6.40 µs | -22% |
| insert/non_crossing/1000 | 77.46 µs | 64.23 µs | -17% |
| insert/non_crossing/10000 | 780.92 µs | 608.91 µs | -22% |

Surprising improvement: with only 2 distinct prices, BTreeMap avoids HashMap's hashing overhead and its pre-allocated 4096-entry bucket array polluting the cache. Regression expected only with many distinct price levels.

### Mixed Workload (Phase 2→3)

| Benchmark | Phase 2 | Phase 3 | Change |
| --- | --- | --- | --- |
| mixed_workload_10k | 825.83 µs | 887.23 µs | +7% |

Slight regression due to BTreeMap node allocation on new price level creation (no `with_capacity` equivalent).

### Test Suite (Phase 2→3)

| Metric | Phase 2 | Phase 3 |
| --- | --- | --- |
| Test count | 53 | 53 |
| Test time | ~0.13s | ~0.11s |

---

## Phase 4: Lock-Free SPSC Ring Buffer

**What changed**: New lock-free single-producer single-consumer ring buffer (`src/ring.rs`) using the Disruptor pattern — free-running cursors with bitmask indexing, `CachePadded` atomics, and local cursor caching.

### Throughput (1M items, 2 threads)

| Benchmark | SPSC Ring | mpsc (recv) | mpsc (try_recv spin) | Speedup vs best mpsc |
| --- | --- | --- | --- | --- |
| u64 / 1M | 1.85 ms | 16.35 ms | 20.04 ms | **8.8x** |
| Order (48B) / 1M | 5.51 ms | 20.03 ms | 24.10 ms | **3.6x** |

SPSC ring buffer: ~540M u64 ops/sec, ~182M Order ops/sec.
std::sync::mpsc (blocking recv): ~61M u64 ops/sec, ~50M Order ops/sec.
std::sync::mpsc (try_recv spin): ~50M u64 ops/sec, ~41M Order ops/sec — spin variant is *slower* than blocking, likely due to `try_recv` lock overhead under contention.

Note: `std::sync::mpsc` is a multi-producer channel (extra synchronization overhead vs our single-producer design). Both benchmarks include thread spawn/join in the measured iteration.

### Latency (single-thread, push 1 / pop 1)

| Benchmark | Time |
| --- | --- |
| push_pop_alternating | 2.11 ns/op |

### Persistence Design

| Property | Value |
| --- | --- |
| Buffer storage | `Box<[UnsafeCell<MaybeUninit<T>>]>` |
| Capacity | Power-of-2, bitmask indexing |
| Cursor strategy | Free-running `AtomicUsize` + mask on access |
| False sharing prevention | `#[repr(align(64))]` `CachePadded<AtomicUsize>` |
| Local cursor caching | Producer caches tail, Consumer caches head |
| Memory ordering | Acquire/Release (plain MOV on x86 TSO) |
| `unsafe` blocks | 4 (slot write, slot read, Send/Sync impls, Drop) |
| New dependencies | 0 |

### Test Suite (Phase 3→4)

| Metric | Phase 3 | Phase 4 |
| --- | --- | --- |
| Test count | 53 | 66 |
| Test time | ~0.11s | ~0.11s |

---

## Phase 5 → Phase 6: Persistence + Deterministic Replay

**What changed**: Added write-ahead log (mmap-backed, CRC32 integrity), periodic snapshots (bincode + serde), and deterministic crash recovery. WAL append integrated into matching loop before matching. Persistence is optional (`data_dir: Option<PathBuf>`).

### WAL Performance

| Benchmark | Time | Notes |
| --- | --- | --- |
| wal/encode_new_order | 56 ns | Single encode + CRC32 |
| wal/encode+crc_10k | 572 µs | 10K NewOrder encodes (57 ns/op) |
| wal/mixed_encode+crc_10k | 498 µs | 80% NewOrder + 20% Cancel (50 ns/op) |

WAL append overhead per order: **~56 ns**. This is the cost added to the hot path when persistence is enabled.

### Snapshot Performance

| Benchmark | Time | Notes |
| --- | --- | --- |
| snapshot/all_resting_orders_10k | 55 µs | Extract 10K orders from book |
| snapshot/bincode_serialize_10k | 71 µs | Serialize 10K orders to bytes |
| snapshot/bincode_deserialize_10k | 163 µs | Deserialize 10K orders from bytes |
| snapshot/restore_from_orders_10k | 1.39 ms | Rebuild engine from 10K orders |

Full snapshot cycle (extract + serialize): **~126 µs** for 10K orders. Happens every 10K commands (default interval), so amortized cost is ~12.6 ns/order.

Recovery from snapshot + 10K WAL replay: **~1.4 ms** worst case.

### Design

| Property | Value |
| --- | --- |
| WAL format | `[len:4][crc32:4][payload:N][pad to 8B align]` |
| WAL payload encoding | Reuses `protocol.rs` `encode_new_order`/`encode_cancel_order` |
| WAL backing | `memmap2::MmapMut` (OS page cache) |
| WAL initial size | 64 MB (~1M records), doubles on growth |
| Snapshot format | `bincode` (serde) |
| Snapshot atomicity | Write to temp file → rename |
| Snapshot naming | `snapshot_{wal_record_count:010}.bin` |
| Integrity | CRC32 per WAL record + per snapshot |
| Recovery | Load latest snapshot → replay WAL from snapshot position |
| Hot-path allocation | Zero (pre-allocated encode buffer in WAL) |
| `unsafe` blocks | 2 (both `MmapMut::map_mut()` with SAFETY comments) |
| New dependencies | `memmap2`, `crc32fast`, `serde`, `bincode` |
| New dev-dependencies | `tempfile` |

### Test Suite (Phase 5→6)

| Metric | Phase 5 | Phase 6 |
| --- | --- | --- |
| Test count | 91 | 134 |
| Test time | ~0.13s | ~0.16s |

---

## Phase 7: Benchmarking Suite + Observability

**What changed**: Eliminated hot-path fills allocation (`AddOrderResult` returns `&[Fill]` instead of `Vec<Fill>`). Added HdrHistogram latency recording, tracking allocator, end-to-end criterion benchmarks, and a load generator that proves the kill metric.

### Fills Allocation Fix

`AddOrderResult.fills` changed from `Vec<Fill>` (heap-allocated via `std::mem::take`) to `&'a [Fill]` (borrowed from pre-allocated buffer). This was the last remaining hot-path allocation in the matching engine.

### End-to-End Latency (loadgen, 1M orders, release build)

| Percentile | Latency |
| --- | --- |
| P50 | 100 ns |
| P90 | 200 ns |
| P99 | 500 ns |
| P99.9 | 800 ns |
| Max | ~38 µs |
| Mean | 151 ns |

**Throughput**: ~4.7M orders/sec (single-threaded matching, ring pop → match → encode).

### End-to-End Criterion Benchmarks

| Benchmark | Time | Per-order |
| --- | --- | --- |
| e2e/crossing_orders/1k | 34 µs | 34 ns/order |
| e2e/crossing_orders/10k | 405 µs | 40 ns/order |
| e2e/mixed_100k | 6.4 ms | 64 ns/op |
| e2e/avg_crossing_100k (100K) | 5.6 ms | 56 ns/order |

### Allocation Tracking

With `--features alloc-track`, the load generator reports heap allocations during the measured phase. The 6 allocations detected are from `BTreeMap` internal node management (level creation/removal) — a known trade-off documented in Phase 3. The matching engine's own data structures (arena, fills buffer) produce zero allocations.

### Test Suite (Phase 6→7)

| Metric | Phase 6 | Phase 7 |
| --- | --- | --- |
| Test count | 134 | 137 (with `--features metrics`) |
| Test time | ~0.16s | ~0.25s |
