#!/usr/bin/env bash
# isolated_kernel.rs rustc body is the term (Aeneas Isolated.lean).
# A Seq view of rustc &[u8] is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-fold/src/isolated_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_isolated_id: verus! stand-in still in isolated_kernel.rs (not last-wins of rustc &[u8])" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_isolated_id: cfg(verus_keep_ghost) split still in isolated_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Isolated.lean"
exit 0
