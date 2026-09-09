#!/usr/bin/env bash
# cf_kernel.rs rustc body is the term (Aeneas Cf.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/src/cf_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_cf_family: verus! stand-in still in cf_kernel.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_cf_family: cfg(verus_keep_ghost) split still in cf_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Cf.lean"
exit 0
