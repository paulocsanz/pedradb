#!/usr/bin/env bash
# prefix.rs rustc body is the term (Aeneas Prefix.lean).
# A Seq/clone_bytes view is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/src/prefix.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_prefix_exclusive_end: verus! stand-in still in prefix.rs (not last-wins of rustc &[u8])" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_prefix_exclusive_end: cfg(verus_keep_ghost) split still in prefix.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Prefix.lean"
exit 0
