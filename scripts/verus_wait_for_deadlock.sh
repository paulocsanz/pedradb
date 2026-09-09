#!/usr/bin/env bash
# locktab.rs rustc body is the term (Aeneas Locktab.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/rocksdb-compat/src/locktab.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_wait_for_deadlock: verus! stand-in still in locktab.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_wait_for_deadlock: cfg(verus_keep_ghost) split still in locktab.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Locktab.lean"
exit 0
