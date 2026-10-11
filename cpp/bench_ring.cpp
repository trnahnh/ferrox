// Throughput / latency harness for cpp/spsc_ring.hpp.
//
// The methodology is identical to examples/ring_throughput.rs so the C++ and
// Rust rings can be compared like-for-like:
//   - u64 and 48-byte Order payloads, 1M items per iteration
//   - one producer thread, consumer on the calling thread, spin-wait on
//     full/empty with a `pause` hint
//   - thread creation and join are inside the timed region (same as the
//     criterion benchmark in benches/ring_bench.rs)
//   - N iterations; min / median / mean reported
//
// Build: g++ -std=c++20 -O3 -pthread cpp/bench_ring.cpp -o cpp/bench_ring
// Run:   cpp/bench_ring [iters=20] [count=1000000] [capacity=8192]
#include "spsc_ring.hpp"

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <numeric>
#include <thread>
#include <vector>

#if defined(__x86_64__) || defined(_M_X64)
#include <immintrin.h>
static inline void spin_hint() { _mm_pause(); }
#else
static inline void spin_hint() {}
#endif

using Clock = std::chrono::steady_clock;

// Same layout and size as ferrox::order::Order (48 bytes).
struct Order {
  std::uint64_t id;
  std::uint64_t trader_id;
  std::int64_t price;
  std::uint64_t quantity;
  std::uint64_t timestamp;
  std::uint8_t side;  // 0 = Bid, 1 = Ask
};
static_assert(sizeof(Order) == 48, "Order must match the Rust struct size");

// Same formula as make_order() in benches/ring_bench.rs.
static Order make_order(std::uint64_t id) {
  return Order{id,
               id % 100,
               10000 + static_cast<std::int64_t>(id % 500),
               (id % 1000) + 1,
               id * 1000,
               static_cast<std::uint8_t>(id % 2 == 0 ? 0 : 1)};
}

struct Stats {
  double min, median, mean;
};

static Stats stats(std::vector<double> v) {
  std::sort(v.begin(), v.end());
  const double mean = std::accumulate(v.begin(), v.end(), 0.0) / v.size();
  const double median = v.size() % 2 == 1
                            ? v[v.size() / 2]
                            : 0.5 * (v[v.size() / 2 - 1] + v[v.size() / 2]);
  return Stats{v.front(), median, mean};
}

static double run_u64(std::uint64_t count, std::size_t capacity) {
  const auto t0 = Clock::now();
  auto [p, c] = ferrox::SpscRing<std::uint64_t>::create(capacity);
  std::thread producer([p = std::move(p), count]() mutable {
    for (std::uint64_t i = 0; i < count; ++i) {
      while (!p.push(i)) spin_hint();
    }
  });
  std::uint64_t sum = 0;
  for (std::uint64_t i = 0; i < count; ++i) {
    std::uint64_t v;
    while (!c.pop(v)) spin_hint();
    sum += v;
  }
  producer.join();
  const auto t1 = Clock::now();
  if (sum != count * (count - 1) / 2) {
    std::fprintf(stderr, "checksum mismatch\n");
    std::exit(1);
  }
  return std::chrono::duration<double, std::milli>(t1 - t0).count();
}

static double run_order(std::uint64_t count, std::size_t capacity) {
  const auto t0 = Clock::now();
  auto [p, c] = ferrox::SpscRing<Order>::create(capacity);
  std::thread producer([p = std::move(p), count]() mutable {
    for (std::uint64_t i = 0; i < count; ++i) {
      Order o = make_order(i);
      while (!p.push(o)) spin_hint();
    }
  });
  std::uint64_t sum = 0;
  for (std::uint64_t i = 0; i < count; ++i) {
    Order o;
    while (!c.pop(o)) spin_hint();
    sum += o.id;
  }
  producer.join();
  const auto t1 = Clock::now();
  if (sum != count * (count - 1) / 2) {
    std::fprintf(stderr, "checksum mismatch\n");
    std::exit(1);
  }
  return std::chrono::duration<double, std::milli>(t1 - t0).count();
}

// Single thread, push one / pop one, ns per push+pop pair.
static double run_latency(std::uint64_t count) {
  auto [p, c] = ferrox::SpscRing<std::uint64_t>::create(1024);
  std::uint64_t sum = 0;
  const auto t0 = Clock::now();
  for (std::uint64_t i = 0; i < count; ++i) {
    p.push(i);
    std::uint64_t v = 0;
    c.pop(v);
    sum += v;
  }
  const auto t1 = Clock::now();
  if (sum != count * (count - 1) / 2) std::exit(1);
  return std::chrono::duration<double, std::nano>(t1 - t0).count() / count;
}

int main(int argc, char** argv) {
  const int iters = argc > 1 ? std::atoi(argv[1]) : 20;
  const std::uint64_t count = argc > 2 ? std::strtoull(argv[2], nullptr, 10) : 1000000;
  const std::size_t capacity = argc > 3 ? std::strtoull(argv[3], nullptr, 10) : 8192;

  std::printf("cpp spsc ring  | ordering=%s padded=%d cursor_cache=%d | iters=%d count=%llu capacity=%zu\n",
              ferrox::kOrderingName, int(ferrox::kCursorsPadded), int(ferrox::kCursorCache),
              iters, static_cast<unsigned long long>(count), capacity);
  std::printf("%-18s %10s %10s %10s %12s\n", "workload", "min_ms", "median_ms", "mean_ms", "median_Mops");

  std::vector<double> u64_ms, order_ms, lat_ns;
  for (int i = 0; i < iters; ++i) u64_ms.push_back(run_u64(count, capacity));
  for (int i = 0; i < iters; ++i) order_ms.push_back(run_order(count, capacity));
  for (int i = 0; i < iters; ++i) lat_ns.push_back(run_latency(count));

  const Stats u = stats(u64_ms), o = stats(order_ms), l = stats(lat_ns);
  std::printf("%-18s %10.3f %10.3f %10.3f %12.1f\n", "u64/1M", u.min, u.median, u.mean,
              count / u.median / 1000.0);
  std::printf("%-18s %10.3f %10.3f %10.3f %12.1f\n", "order48/1M", o.min, o.median, o.mean,
              count / o.median / 1000.0);
  std::printf("%-18s %10.3f %10.3f %10.3f %12s\n", "push_pop_ns/op", l.min, l.median, l.mean, "-");
  return 0;
}
