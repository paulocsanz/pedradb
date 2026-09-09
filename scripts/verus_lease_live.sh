#!/usr/bin/env bash
# lease_kernel.rs rustc body is the term (Aeneas Lease.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-dcs/src/lease_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_lease_live: verus! stand-in still in lease_kernel.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_lease_live: cfg(verus_keep_ghost) split still in lease_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Lease.lean"
exit 0
