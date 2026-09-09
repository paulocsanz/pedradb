#!/usr/bin/env bash
# leveling.rs rustc body is the term (Aeneas Leveling.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/src/leveling.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_leveling: verus! stand-in still in leveling.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_leveling: cfg(verus_keep_ghost) split still in leveling.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Leveling.lean"
exit 0
