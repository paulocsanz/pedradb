#!/usr/bin/env bash
# iter_kernel.rs rustc body is the term (Aeneas Iter.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/rocksdb-compat/src/iter_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_iter_window: verus! stand-in still in iter_kernel.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_iter_window: cfg(verus_keep_ghost) split still in iter_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Iter.lean"
exit 0
