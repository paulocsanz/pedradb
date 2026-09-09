#!/usr/bin/env bash
# merge.rs rustc body is the term (Aeneas Merge.lean).
# A Verus u64/toy-enum view is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/src/merge.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_visible_at: verus! stand-in still in merge.rs (not last-wins of rustc types)" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_visible_at: cfg(verus_keep_ghost) split still in merge.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Merge.lean"
exit 0
