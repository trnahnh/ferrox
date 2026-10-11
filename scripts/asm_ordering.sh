#!/usr/bin/env bash
# Emits the x86-64 code generated for the ring buffer's push/pop in Rust and C++
# under (a) the shipped acquire/release ordering and (b) SeqCst, then counts the
# fence-like instructions (xchg / mfence / lock-prefixed) inside those functions.
#
# This is the evidence behind the "Acquire/Release compiles to plain MOV on x86"
# claim: see docs/MEMORY_ORDERING.md.
#
# Usage: scripts/asm_ordering.sh [out_dir=docs/asm]
# Needs: rustc (stable), g++ (any C++20 compiler; clang++ works too via CXX=).
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=${1:-docs/asm}
CXX=${CXX:-g++}
mkdir -p "$OUT"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# Extract one function body (label .. ret) from a .s file, dropping directives.
extract() { # file symbol_substring
  # From the function's label to its end-of-procedure directive (Windows SEH or
  # Linux CFI) or the next .size directive, whichever comes first. Must not stop
  # at the first `ret`: the early-return (full/empty) path precedes the store.
  awk -v sym="$2" '
    !on && $0 ~ sym && /:$/ { on = 1 }
    on && /\.seh_endproc|\.cfi_endproc|^\s*\.size/ { exit }
    on { print }
  ' "$1" | grep -vE '^\s*\.(seh|cfi|p2align|def|scl|type|endef|size|section|text|globl|file|loc|ident|addrsig|L[A-Z]+)' || true
}

census() { # file -> "xchg=N mfence=N lock=N"
  printf 'xchg=%s mfence=%s lock=%s' \
    "$(grep -c 'xchg' "$1" || true)" "$(grep -c 'mfence' "$1" || true)" "$(grep -c 'lock ' "$1" || true)"
}

echo "== Rust (rustc $(rustc --version | cut -d' ' -f2), -O, target-cpu=x86-64) =="
rustc -O --edition 2024 --emit asm -C target-cpu=x86-64 scripts/asm_probe.rs -o "$TMP/rust_acqrel.s" 2>/dev/null
rustc -O --edition 2024 --emit asm -C target-cpu=x86-64 --cfg ring_seqcst scripts/asm_probe.rs -o "$TMP/rust_seqcst.s" 2>/dev/null
for v in acqrel seqcst; do
  { extract "$TMP/rust_$v.s" 'push_u64'; echo; extract "$TMP/rust_$v.s" 'pop_u64'; } > "$OUT/rust_ring_$v.s"
  echo "rust $v push+pop: $(census "$OUT/rust_ring_$v.s")"
done

echo "== C++ ($CXX $($CXX -dumpversion), -O3, -std=c++20) =="
$CXX -std=c++20 -O3 -S -o "$TMP/cpp_acqrel.s" cpp/asm_probe.cpp
$CXX -std=c++20 -O3 -S -DFERROX_SEQ_CST -o "$TMP/cpp_seqcst.s" cpp/asm_probe.cpp
for v in acqrel seqcst; do
  { extract "$TMP/cpp_$v.s" 'probe_push'; echo; extract "$TMP/cpp_$v.s" 'probe_pop'; } > "$OUT/cpp_ring_$v.s"
  echo "c++  $v push+pop: $(census "$OUT/cpp_ring_$v.s")"
done

echo "wrote $OUT/{rust,cpp}_ring_{acqrel,seqcst}.s"
