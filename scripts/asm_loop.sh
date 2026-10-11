#!/usr/bin/env bash
# Emits the x86-64 code of the single-thread push+pop loop (the
# `push_pop_ns/op` workload) for the Rust ring and the C++ port, with the
# optional knobs, and counts the instructions in the hot loop body.
#
# This is the evidence behind the single-thread Rust-vs-C++ gap analysis in
# docs/MEMORY_ORDERING.md §5.
#
# Usage: scripts/asm_loop.sh [out_dir=docs/asm]
#   CXX=clang++ scripts/asm_loop.sh      # second C++ compiler
#   RUST_CFG="--cfg ring_unchecked_index" scripts/asm_loop.sh
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=${1:-docs/asm}
CXX=${CXX:-g++}
RUST_CFG=${RUST_CFG:-}
mkdir -p "$OUT"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

extract() { # file symbol_substring
  awk -v sym="$2" '
    !on && $0 ~ sym && /:$/ { on = 1 }
    on && /\.seh_endproc|\.cfi_endproc|^\s*\.size/ { exit }
    on { print }
  ' "$1" | grep -vE '^\s*\.(seh|cfi|p2align|def|scl|type|endef|size|section|text|globl|file|loc|ident|addrsig|long|uleb128|byte)[ _\t]' || true
}

# Number of instructions between the innermost backward-branch target and the
# branch itself: a crude but compiler-neutral "loop body length".
loop_len() { # file
  awk '
    /^[.A-Za-z_][^ \t]*:$/ { label = $0; sub(/:$/, "", label); start[label] = n; next }
    /^\s*(j[a-z]+|jmp)\s/ { split($0, f, /[ \t]+/); tgt = f[length(f)]; if (tgt in start) { len = n - start[tgt] + 1; if (best == 0 || len < best) best = len } }
    /^\s*[a-z]/ { n++ }
    END { print best + 0 }
  ' "$1"
}

tag=${RUST_CFG//--cfg /}; tag=${tag// /_}; tag=${tag:-default}
echo "== Rust (rustc $(rustc --version | cut -d' ' -f2), -O, target-cpu=x86-64, cfg: ${RUST_CFG:-none}) =="
rustc -O --edition 2024 --emit asm -C target-cpu=x86-64 $RUST_CFG scripts/asm_probe.rs -o "$TMP/rust.s" 2>/dev/null
for fn in latency_loop latency_loop_nounwrap latency_bench; do
  extract "$TMP/rust.s" "$fn" > "$OUT/rust_${fn}_${tag}.s"
  echo "rust $fn ($tag): $(grep -cE '^\s*[a-z]' "$OUT/rust_${fn}_${tag}.s") instructions, innermost loop $(loop_len "$OUT/rust_${fn}_${tag}.s")"
done

cxxname=$(basename "$CXX")
echo "== C++ ($cxxname $($CXX -dumpversion), -O3, -std=c++20) =="
$CXX -std=c++20 -O3 -S -o "$TMP/cpp.s" cpp/asm_probe.cpp
for fn in latency_loop latency_bench; do
  extract "$TMP/cpp.s" "$fn" > "$OUT/cpp_${fn}_${cxxname}.s"
  echo "c++  $fn ($cxxname): $(grep -cE '^\s*[a-z]' "$OUT/cpp_${fn}_${cxxname}.s") instructions, innermost loop $(loop_len "$OUT/cpp_${fn}_${cxxname}.s")"
done

echo "wrote $OUT/rust_latency_loop*_${tag}.s and $OUT/cpp_latency_loop_${cxxname}.s"
