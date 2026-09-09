#!/usr/bin/env bash
# apply_kernel.rs rustc body is the term (Aeneas Apply.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-raft/src/apply_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_apply_advance: verus! stand-in still in apply_kernel.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_apply_advance: cfg(verus_keep_ghost) split still in apply_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Apply.lean"
exit 0
