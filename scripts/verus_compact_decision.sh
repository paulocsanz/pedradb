#!/usr/bin/env bash
# compact_kernel.rs rustc body is the term (Aeneas Compact.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/src/compact_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_compact_decision: verus! stand-in still in compact_kernel.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_compact_decision: cfg(verus_keep_ghost) split still in compact_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Compact.lean"
exit 0
