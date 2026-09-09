#!/usr/bin/env bash
# leveling.rs rustc pick body is the term (Aeneas Leveling.lean).
# A Verus u64-key stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/src/leveling.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_leveling_pick: verus! stand-in still in leveling.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_leveling_pick: cfg(verus_keep_ghost) split still in leveling.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Leveling.lean"
exit 0
