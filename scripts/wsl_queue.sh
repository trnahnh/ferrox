#!/usr/bin/env bash
# Sequential Linux verification queue for Ferrox round 2, run inside WSL2.
# The VM has ~200 MB free (Docker Desktop containers hold the rest), so every
# step runs alone with -j1 and its own log. Nothing here writes into the repo
# except through cargo's normal behaviour; CARGO_TARGET_DIR is on the Linux disk.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$HOME/ferrox-target"
export CARGO_BUILD_JOBS=1
export CARGO_TERM_COLOR=never
REPO=/mnt/d/Projects/Trading-Matching-Engine/ferrox
LOGS="$HOME/ferrox-build/logs"
mkdir -p "$LOGS"
cd "$REPO"

step() { # name cmd...
  local name=$1; shift
  echo "[$(date +%T)] START $name" | tee -a "$LOGS/queue.log"
  ( "$@" ) > "$LOGS/$name.log" 2>&1
  local rc=$?
  echo "[$(date +%T)] END   $name rc=$rc" | tee -a "$LOGS/queue.log"
  return $rc
}

# Wait for the rustup install and any clang job started earlier to finish.
while pgrep -x rustup >/dev/null || pgrep -f 'test_tsan|test_asan|clang\+\+' >/dev/null; do sleep 20; done
echo "[$(date +%T)] toolchains: $(ls ~/.rustup/toolchains | tr '\n' ' ')" | tee -a "$LOGS/queue.log"
rustc --version | tee -a "$LOGS/queue.log"; rustc +nightly --version | tee -a "$LOGS/queue.log"

# 1. Miri on the ring's unsafe slot code (single-threaded tests only).
step miri_setup bash -c 'cargo +nightly miri setup'
step miri bash -c 'MIRIFLAGS="-Zmiri-strict-provenance" cargo +nightly miri test --lib ring:: -- --skip concurrent_'

# 2. loom, shipped ordering must pass; Relaxed must be caught.
step loom bash -c 'RUSTFLAGS="--cfg loom" cargo test --release --test loom_ring'
step loom_relaxed_negative bash -c 'if RUSTFLAGS="--cfg loom --cfg ring_relaxed_ordering" cargo test --release --test loom_ring; then echo "NEGATIVE CONTROL FAILED: relaxed ordering passed loom"; exit 1; else echo "negative control OK: loom rejected relaxed ordering"; fi'

# 3. Stable test suite on Linux, including the crash harness (process kill on Linux).
step test_all bash -c 'cargo test --release --features metrics -- --nocapture --test-threads=1 2>&1 | grep -vE "^\s*(Compiling|Downloaded|Downloading)"'

# 4. ThreadSanitizer on the Rust ring (needs -Zbuild-std; heaviest build).
step tsan_rust bash -c 'RUSTFLAGS="-Zsanitizer=thread" RUSTDOCFLAGS="-Zsanitizer=thread" cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu --lib ring:: -- --test-threads=1'

# 5. cargo-fuzz: install, then 10 minutes per target, then coverage for the protocol decoder.
step fuzz_install bash -c 'cargo +nightly install cargo-fuzz --locked 2>&1 | tail -5; cargo +nightly fuzz --version'
for t in protocol_decode wal_recover snapshot_load; do
  step "fuzz_$t" bash -c "cargo +nightly fuzz run $t -- -max_total_time=600 -print_final_stats=1 2>&1 | tail -60"
done
step fuzz_coverage bash -c 'cargo +nightly fuzz coverage protocol_decode 2>&1 | tail -5; SYSROOT=$(rustc +nightly --print sysroot); COV=$(ls $SYSROOT/lib/rustlib/*/bin/llvm-cov | head -1); $COV report --use-color=false --instr-profile=fuzz/coverage/protocol_decode/coverage.profdata --ignore-filename-regex="/(\.cargo|rustc|rustlib)/" "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/coverage/x86_64-unknown-linux-gnu/release/protocol_decode" 2>&1 | tail -40'

# 6. Hardware counters.
step perf bash -c 'PERF=$(ls /usr/lib/linux-tools/*/perf | head -1) bash scripts/perf_counters.sh 2,4 5 1000000 5'

# 7. Pinned Rust-vs-C++ and knob variance study (binaries built above or here).
step build_cpp bash -c 'g++ -std=c++20 -O3 -pthread cpp/bench_ring.cpp -o ~/ferrox-build/bench_ring_gcc && clang++ -std=c++20 -O3 -pthread cpp/bench_ring.cpp -o ~/ferrox-build/bench_ring_clang && echo built'
step build_rust_default bash -c 'cargo build --release --example ring_throughput && cp "$CARGO_TARGET_DIR/release/examples/ring_throughput" ~/ferrox-build/ring_throughput_default'
step build_rust_unchecked bash -c 'RUSTFLAGS="--cfg ring_unchecked_index" cargo build --release --example ring_throughput && cp "$CARGO_TARGET_DIR/release/examples/ring_throughput" ~/ferrox-build/ring_throughput_unchecked'
step variance_rust_vs_cpp bash -c 'python3 scripts/bench_compare.py --pin 2,4 --rounds 10 --iters 4 --json ~/ferrox-build/variance_lang.json rust=$HOME/ferrox-build/ring_throughput_default cpp_gcc=$HOME/ferrox-build/bench_ring_gcc cpp_clang=$HOME/ferrox-build/bench_ring_clang rust_unchecked=$HOME/ferrox-build/ring_throughput_unchecked'
step variance_unpinned bash -c 'python3 scripts/bench_compare.py --rounds 10 --iters 4 --json ~/ferrox-build/variance_unpinned.json rust=$HOME/ferrox-build/ring_throughput_default cpp_gcc=$HOME/ferrox-build/bench_ring_gcc'

echo "[$(date +%T)] QUEUE DONE" | tee -a "$LOGS/queue.log"
