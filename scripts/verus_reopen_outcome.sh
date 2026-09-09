#!/usr/bin/env bash
# wal/reopen_kernel.rs rustc body is the term (Aeneas Reopen.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/src/wal/reopen_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_reopen_outcome: verus! stand-in still in reopen_kernel.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_reopen_outcome: cfg(verus_keep_ghost) split still in reopen_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Reopen.lean"
exit 0
