#!/usr/bin/env bash
# Packed-children exclusive end (RFC-0002 P23 / F59).
# The rustc body is the term (Aeneas extract). A Verus stand-in on this
# file is a cartoon — refuse it.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/montanha-fdb-recipes/src/children_kernel.rs"
if grep -nE 'verus![[:space:]]*\{|#\[cfg\((not\()?verus_keep_ghost' "$SRC" >/dev/null; then
  echo "FAIL  $SRC still has a verus! stand-in; Aeneas of the rustc body is the term" >&2
  exit 1
fi
if ! grep -q 'fn packed_children_end' "$SRC"; then
  echo "FAIL  $SRC missing rustc packed_children_end" >&2
  exit 1
fi
echo "ok    no verus! stand-in; rustc children_kernel.rs is the term"
exit 0
