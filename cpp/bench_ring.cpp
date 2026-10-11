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
// Run:   cpp/bench_ring [iters=20] [count=1000000] [capacity=8192] [flags]
//
// Flags (same as the Rust harness):
//   --pin P,C      pin the producer to CPU P and the consumer (main thread) to CPU C
//   --raw          print every trial as `raw <workload> <value>`
//   --workload W   u64 | order | latency | all
#include "spsc_ring.hpp"

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <numeric>
#include <optional>
#include <string>
#include <thread>
#include <vector>

#if defined(__x86_64__) || defined(_M_X64)
#include <immintrin.h>
static inline void spin_hint() { _mm_pause(); }
#else
static inline void spin_hint() {}
#endif

#if defined(__linux__)
#include <sched.h>
static bool pin_current_thread(int cpu) {
  cpu_set_t set;
  CPU_ZERO(&set);
  CPU_SET(cpu, &set);
  return sched_setaffinity(0, sizeof(set), &set) == 0;
}
#elif defined(_WIN32)
#include <windows.h>
static bool pin_current_thread(int cpu) {
  return SetThreadAffinityMask(GetCurrentThread(), DWORD_PTR(1) << cpu) != 0;
}
#else
static bool pin_current_thread(int) { return false; }
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

struct Config {
  std::uint64_t count;
  std::size_t capacity;
  std::optional<int> producer_cpu;
};

static void pin_producer(const Config& cfg) {
  if (cfg.producer_cpu && !pin_current_thread(*cfg.producer_cpu)) {
    std::fprintf(stderr, "failed to pin producer to cpu %d\n", *cfg.producer_cpu);
    std::exit(1);
  }
}

static double run_u64(const Config& cfg) {
  const std::uint64_t count = cfg.count;
  const auto t0 = Clock::now();
  auto [p, c] = ferrox::SpscRing<std::uint64_t>::create(cfg.capacity);
  std::thread producer([p = std::move(p), count, cfg]() mutable {
    pin_producer(cfg);
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

static double run_order(const Config& cfg) {
  const std::uint64_t count = cfg.count;
  const auto t0 = Clock::now();
  auto [p, c] = ferrox::SpscRing<Order>::create(cfg.capacity);
  std::thread producer([p = std::move(p), count, cfg]() mutable {
    pin_producer(cfg);
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

static void print_row(const char* name, const Stats& s, double mops) {
  if (mops > 0) {
    std::printf("%-18s %10.3f %10.3f %10.3f %12.1f\n", name, s.min, s.median, s.mean, mops);
  } else {
    std::printf("%-18s %10.3f %10.3f %10.3f %12s\n", name, s.min, s.median, s.mean, "-");
  }
}

int main(int argc, char** argv) {
  std::vector<std::string> positional;
  std::optional<std::pair<int, int>> pin;
  bool raw = false;
  std::string workload = "all";
  for (int i = 1; i < argc; ++i) {
    std::string a = argv[i];
    if (a == "--pin" && i + 1 < argc) {
      std::string spec = argv[++i];
      auto comma = spec.find(',');
      if (comma == std::string::npos) {
        std::fprintf(stderr, "--pin needs P,C\n");
        return 2;
      }
      pin = std::make_pair(std::stoi(spec.substr(0, comma)), std::stoi(spec.substr(comma + 1)));
    } else if (a == "--raw") {
      raw = true;
    } else if (a == "--workload" && i + 1 < argc) {
      workload = argv[++i];
    } else {
      positional.push_back(a);
    }
  }
  const int iters = positional.size() > 0 ? std::atoi(positional[0].c_str()) : 20;
  const std::uint64_t count =
      positional.size() > 1 ? std::strtoull(positional[1].c_str(), nullptr, 10) : 1000000;
  const std::size_t capacity =
      positional.size() > 2 ? std::strtoull(positional[2].c_str(), nullptr, 10) : 8192;

  if (pin && !pin_current_thread(pin->second)) {
    std::fprintf(stderr, "failed to pin consumer to cpu %d\n", pin->second);
    return 1;
  }
  Config cfg{count, capacity, pin ? std::optional<int>(pin->first) : std::nullopt};

  std::printf(
      "cpp spsc ring  | ordering=%s padded=%d cursor_cache=%d | iters=%d count=%llu capacity=%zu pin=%s\n",
      ferrox::kOrderingName, int(ferrox::kCursorsPadded), int(ferrox::kCursorCache), iters,
      static_cast<unsigned long long>(count), capacity,
      pin ? (std::to_string(pin->first) + "," + std::to_string(pin->second)).c_str() : "none");

  auto want = [&](const char* w) { return workload == "all" || workload == w; };
  auto collect = [&](const char* name, auto&& fn) -> std::optional<std::vector<double>> {
    if (!want(name)) return std::nullopt;
    std::vector<double> v;
    v.reserve(iters);
    for (int i = 0; i < iters; ++i) {
      const double x = fn();
      if (raw) {
        std::printf("raw %s %.6f\n", name, x);
        std::fflush(stdout);
      }
      v.push_back(x);
    }
    return v;
  };

  auto u64_ms = collect("u64", [&] { return run_u64(cfg); });
  auto order_ms = collect("order", [&] { return run_order(cfg); });
  auto lat_ns = collect("latency", [&] { return run_latency(count); });

  std::printf("%-18s %10s %10s %10s %12s\n", "workload", "min_ms", "median_ms", "mean_ms", "median_Mops");
  if (u64_ms) {
    const Stats s = stats(*u64_ms);
    print_row("u64/1M", s, count / s.median / 1000.0);
  }
  if (order_ms) {
    const Stats s = stats(*order_ms);
    print_row("order48/1M", s, count / s.median / 1000.0);
  }
  if (lat_ns) print_row("push_pop_ns/op", stats(*lat_ns), 0);
  return 0;
}
