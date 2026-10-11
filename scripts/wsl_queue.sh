#!/usr/bin/env bash
# Sequential Linux verification queue for Ferrox round 2, run inside WSL2.
#
# The VM on the dev machine has ~200 MB free (Docker Desktop containers hold
# the rest of its 3.9 GB), so every step runs alone with -j1, writes its own
# log, and the steps that need only the stable toolchain come first; the
# nightly toolchain (Miri, TSan, cargo-fuzz) is installed and used last.
# CARGO_TARGET_DIR is on the Linux disk; nothing is written into the repo
# except by cargo-fuzz (corpus).
#
#   nohup bash scripts/wsl_queue.sh > ~/ferrox-build/logs/nohup.out 2>&1 &
#   tail -f ~/ferrox-build/logs/queue.log
set -u
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$HOME/ferrox-target"
export CARGO_BUILD_JOBS=1
export CARGO_TERM_COLOR=never
export RUSTUP_UNPACK_RAM=200000000 RUSTUP_IO_THREADS=1
REPO=${REPO:-/mnt/d/Projects/Trading-Matching-Engine/ferrox}
LOGS="$HOME/ferrox-build/logs"
BIN="$HOME/ferrox-build"
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

echo "[$(date +%T)] $(rustc --version) | $(uname -r) | load $(cut -d' ' -f1-3 /proc/loadavg)" | tee -a "$LOGS/queue.log"

# ---- stable toolchain ------------------------------------------------------

# 1. loom: shipped ordering must pass, Relaxed must be caught.
step loom bash -c 'RUSTFLAGS="--cfg loom" cargo test --release --test loom_ring'
step loom_relaxed_negative bash -c 'if RUSTFLAGS="--cfg loom --cfg ring_relaxed_ordering" cargo test --release --test loom_ring; then echo "NEGATIVE CONTROL FAILED: relaxed ordering passed loom"; exit 1; else echo "negative control OK: loom rejected relaxed ordering"; fi'

# 2. Hardware counters (perf stat) for the four ring variants (builds the example 4 times).
step perf bash -c 'PERF=$(ls /usr/lib/linux-tools/*/perf | head -1) bash scripts/perf_counters.sh 2,4 5 1000000 5'

# 3. Pinned Rust-vs-C++ (and knob) variance study.
step build_cpp bash -c "g++ -std=c++20 -O3 -pthread cpp/bench_ring.cpp -o $BIN/bench_ring_gcc && clang++ -std=c++20 -O3 -pthread cpp/bench_ring.cpp -o $BIN/bench_ring_clang && echo built"
step build_rust_default bash -c "cargo build --release --example ring_throughput && cp \$CARGO_TARGET_DIR/release/examples/ring_throughput $BIN/ring_throughput_default"
step build_rust_unchecked bash -c "RUSTFLAGS='--cfg ring_unchecked_index' cargo build --release --example ring_throughput && cp \$CARGO_TARGET_DIR/release/examples/ring_throughput $BIN/ring_throughput_unchecked"
step variance_pinned bash -c "python3 scripts/bench_compare.py --pin 2,4 --rounds 10 --iters 4 --json $BIN/variance_pinned.json rust=$BIN/ring_throughput_default cpp_gcc=$BIN/bench_ring_gcc cpp_clang=$BIN/bench_ring_clang rust_unchecked=$BIN/ring_throughput_unchecked"
step variance_unpinned bash -c "python3 scripts/bench_compare.py --rounds 10 --iters 4 --json $BIN/variance_unpinned.json rust=$BIN/ring_throughput_default cpp_gcc=$BIN/bench_ring_gcc"

# 4. Full stable test suite on Linux, including the crash harness (process kill on Linux).
step test_all bash -c 'cargo test --release --features metrics -- --nocapture --test-threads=1 2>&1 | grep -vE "^\s*(Compiling|Downloaded|Downloading)"'

# ---- nightly toolchain ------------------------------------------------------

step nightly_install bash -c 'rustup toolchain install nightly --profile minimal && rustup component add --toolchain nightly miri rust-src llvm-tools && rustc +nightly --version && cargo +nightly miri --version'

# 5. Miri on the ring (zero-dependency crate; the 100k-item concurrent tests are skipped as too slow to interpret).
step miri bash -c 'cd verify/ring-standalone && MIRIFLAGS="-Zmiri-strict-provenance" cargo +nightly miri test -- --skip concurrent_'

# 6. ThreadSanitizer on the real concurrent ring tests (rebuilds std with TSan).
step tsan_rust bash -c 'cd verify/ring-standalone && RUSTFLAGS="-Zsanitizer=thread" cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu -- --test-threads=1'
step tsan_rust_relaxed_negative bash -c 'cd verify/ring-standalone && if RUSTFLAGS="-Zsanitizer=thread --cfg ring_relaxed_ordering --cfg ring_allow_unsound" cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu -- --test-threads=1 concurrent_ 2>&1 | tee /dev/stderr | grep -q "ThreadSanitizer: data race"; then echo "negative control OK: TSan reported the race"; else echo "NEGATIVE CONTROL: TSan did not report a race (or the build failed; see above)"; exit 1; fi'

# 7. cargo-fuzz: install, 10 minutes per target, coverage for the protocol decoder.
step fuzz_install bash -c 'cargo +nightly install cargo-fuzz --locked 2>&1 | tail -3; cargo +nightly fuzz --version'
for t in protocol_decode wal_recover snapshot_load; do
  step "fuzz_$t" bash -c "cargo +nightly fuzz run $t -- -max_total_time=600 -print_final_stats=1 2>&1 | tail -60"
done
step fuzz_coverage bash -c 'cargo +nightly fuzz coverage protocol_decode 2>&1 | tail -3; SYSROOT=$(rustc +nightly --print sysroot); COV=$(ls $SYSROOT/lib/rustlib/*/bin/llvm-cov | head -1); $COV report --use-color=false --instr-profile=fuzz/coverage/protocol_decode/coverage.profdata --ignore-filename-regex="/(\.cargo|rustc|rustlib)/" "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/coverage/x86_64-unknown-linux-gnu/release/protocol_decode" 2>&1 | tail -40'

echo "[$(date +%T)] QUEUE DONE" | tee -a "$LOGS/queue.log"
