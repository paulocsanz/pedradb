#!/usr/bin/env bash
# ae_kernel.rs rustc body is the term (Aeneas Ae.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-raft/src/ae_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_ae_entry_action: verus! stand-in still in ae_kernel.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_ae_entry_action: cfg(verus_keep_ghost) split still in ae_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Ae.lean"
exit 0
