// Correctness tests for cpp/spsc_ring.hpp. Mirrors the unit tests in src/ring.rs.
// Build: g++ -std=c++20 -O2 -pthread cpp/test_ring.cpp -o cpp/test_ring
#include "spsc_ring.hpp"

#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <thread>
#include <vector>

#define CHECK(cond)                                                       \
  do {                                                                    \
    if (!(cond)) {                                                        \
      std::fprintf(stderr, "CHECK failed: %s (%s:%d)\n", #cond, __FILE__, \
                   __LINE__);                                             \
      std::exit(1);                                                       \
    }                                                                     \
  } while (0)

using ferrox::SpscRing;

static void push_pop_fifo() {
  auto [p, c] = SpscRing<std::uint64_t>::create(8);
  for (std::uint64_t i = 0; i < 8; ++i) CHECK(p.push(i));
  CHECK(!p.push(std::uint64_t{99}));  // full
  for (std::uint64_t i = 0; i < 8; ++i) {
    std::uint64_t v = 0;
    CHECK(c.pop(v));
    CHECK(v == i);
  }
  std::uint64_t v = 0;
  CHECK(!c.pop(v));  // empty
}

static void wraparound() {
  auto [p, c] = SpscRing<std::uint64_t>::create(4);
  for (std::uint64_t i = 0; i < 100; ++i) {
    CHECK(p.push(i));
    std::uint64_t v = 0;
    CHECK(c.pop(v));
    CHECK(v == i);
  }
}

static void rejects_bad_capacity() {
  bool threw = false;
  try {
    (void)SpscRing<int>::create(3);
  } catch (const std::invalid_argument&) {
    threw = true;
  }
  CHECK(threw);
  threw = false;
  try {
    (void)SpscRing<int>::create(0);
  } catch (const std::invalid_argument&) {
    threw = true;
  }
  CHECK(threw);
}

struct DropCounter {
  static std::atomic<int> live;
  DropCounter() { live.fetch_add(1); }
  DropCounter(const DropCounter&) { live.fetch_add(1); }
  DropCounter(DropCounter&&) noexcept { live.fetch_add(1); }
  DropCounter& operator=(const DropCounter&) = default;
  DropCounter& operator=(DropCounter&&) noexcept = default;
  ~DropCounter() { live.fetch_sub(1); }
};
std::atomic<int> DropCounter::live{0};

static void destroys_unconsumed_items() {
  {
    auto [p, c] = SpscRing<DropCounter>::create(4);
    CHECK(p.push(DropCounter{}));
    CHECK(p.push(DropCounter{}));
    CHECK(p.push(DropCounter{}));
    CHECK(DropCounter::live.load() == 3);
  }
  CHECK(DropCounter::live.load() == 0);
}

static void concurrent_fifo(std::size_t capacity, std::uint64_t count) {
  auto [p, c] = SpscRing<std::uint64_t>::create(capacity);
  std::thread producer([p = std::move(p), count]() mutable {
    for (std::uint64_t i = 0; i < count; ++i) {
      while (!p.push(i)) std::this_thread::yield();
    }
  });
  std::vector<std::uint64_t> received;
  received.reserve(count);
  while (received.size() < count) {
    std::uint64_t v = 0;
    if (c.pop(v)) {
      received.push_back(v);
    } else {
      std::this_thread::yield();
    }
  }
  producer.join();
  for (std::uint64_t i = 0; i < count; ++i) CHECK(received[i] == i);
}

int main() {
  push_pop_fifo();
  wraparound();
  rejects_bad_capacity();
  destroys_unconsumed_items();
  concurrent_fifo(1024, 100000);  // mirrors concurrent_push_pop
  concurrent_fifo(16, 100000);    // mirrors concurrent_backpressure
  std::printf("cpp/test_ring: 6 tests ok (ordering=%s, padded=%d, cursor_cache=%d)\n",
              ferrox::kOrderingName, int(ferrox::kCursorsPadded),
              int(ferrox::kCursorCache));
  return 0;
}
