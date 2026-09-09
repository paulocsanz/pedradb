#!/usr/bin/env bash
# membership_kernel.rs rustc body is the term (Aeneas Membership.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-raft/src/membership_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_membership_joint: verus! stand-in still in membership_kernel.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_membership_joint: cfg(verus_keep_ghost) split still in membership_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Membership.lean"
exit 0
