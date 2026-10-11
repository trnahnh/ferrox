#!/usr/bin/env bash
# Hardware-counter comparison of the ring-buffer variants with `perf stat`.
#
# For each variant (shipped, no padding = false sharing, SeqCst, no cursor
# cache) this builds the Rust harness with the matching --cfg knob and runs
# the u64 cross-thread workload under perf stat, pinned, several times.
# Linux only (perf is a Linux tool). On WSL2 the PMU may or may not be
# exposed; the script prints exactly what perf reports, including
# "<not supported>", so an absent counter is visible rather than silent.
#
# Usage: bash scripts/perf_counters.sh [pin=P,C] [repeats=5] [count=1000000] [iters=5]
#   PERF=/path/to/perf  overrides the perf binary (on WSL2 Ubuntu the wrapper
#   script refuses to run for a mismatched kernel; the real binary lives in
#   /usr/lib/linux-tools/<version>/perf).
set -euo pipefail
cd "$(dirname "$0")/.."

PIN=${1:-2,4}
REPEATS=${2:-5}
COUNT=${3:-1000000}
ITERS=${4:-5}
PERF=${PERF:-$(ls /usr/lib/linux-tools/*/perf 2>/dev/null | head -1 || true)}
PERF=${PERF:-perf}
OUT=${OUT:-docs/perf}
mkdir -p "$OUT"

EVENTS="cycles,instructions,cache-references,cache-misses,LLC-loads,LLC-load-misses,branch-instructions,branch-misses,context-switches,cpu-migrations"

echo "perf: $($PERF --version 2>&1 | head -1)"
echo "kernel: $(uname -r)   perf_event_paranoid: $(cat /proc/sys/kernel/perf_event_paranoid)"
echo "pin=$PIN repeats=$REPEATS count=$COUNT iters=$ITERS"
echo

declare -A CFG=(
  [shipped]=""
  [no_padding]="--cfg ring_no_padding"
  [seqcst]="--cfg ring_seqcst"
  [no_cursor_cache]="--cfg ring_no_cursor_cache"
)

for name in shipped no_padding seqcst no_cursor_cache; do
  echo "=== $name  (RUSTFLAGS=\"${CFG[$name]}\")"
  RUSTFLAGS="${CFG[$name]}" cargo build --release --example ring_throughput -q 2>&1 | grep -v "^warning" || true
  bin=${CARGO_TARGET_DIR:-target}/release/examples/ring_throughput
  # -r repeats the whole process and reports mean +- stddev per counter;
  # -x, gives machine-readable CSV that is saved next to the human output.
  "$PERF" stat -r "$REPEATS" -e "$EVENTS" -x, -o "$OUT/${name}.csv" -- \
      "$bin" "$ITERS" "$COUNT" 8192 --workload u64 --pin "$PIN" > "$OUT/${name}.stdout"
  grep -E "^(u64|rust)" "$OUT/${name}.stdout"
  # CSV columns: value,unit,event,run-time,pct-running,derived,derived-unit
  awk -F, 'NF>=3 && $1 !~ /^#/ { printf "  %-22s %18s %s\n", $3, $1, ($6 != "" ? "(" $6 " " $7 ")" : "") }' "$OUT/${name}.csv"
  echo
done

echo "raw perf output in $OUT/*.csv and harness output in $OUT/*.stdout"
