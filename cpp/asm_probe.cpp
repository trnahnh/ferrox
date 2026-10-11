// Minimal translation unit whose only purpose is to expose the ring's push/pop
// as out-of-line functions so their generated x86-64 code can be inspected.
// Used by scripts/asm_ordering.sh; see docs/MEMORY_ORDERING.md for the results.
//
//   g++ -std=c++20 -O3 -S cpp/asm_probe.cpp -o probe_acqrel.s
//   g++ -std=c++20 -O3 -S -DFERROX_SEQ_CST cpp/asm_probe.cpp -o probe_seqcst.s
#include "spsc_ring.hpp"

#include <cstdint>

using Ring = ferrox::SpscRing<std::uint64_t>;

__attribute__((noinline)) bool probe_push(Ring::Producer& p, std::uint64_t v) {
  return p.push(v);
}

__attribute__((noinline)) std::uint64_t probe_pop(Ring::Consumer& c) {
  std::uint64_t v = 0;
  c.pop(v);
  return v;
}

int main(int argc, char**) {
  auto [p, c] = Ring::create(1024);
  probe_push(p, static_cast<std::uint64_t>(argc));
  return static_cast<int>(probe_pop(c));
}
