#!/usr/bin/env bash
# properties_kernel.rs rustc body is the term (Aeneas Properties.lean).
# A Seq view of rustc &[bool] is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-spec/src/properties_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_spec_properties: verus! stand-in still in properties_kernel.rs (not last-wins of rustc &[bool])" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_spec_properties: cfg(verus_keep_ghost) split still in properties_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Properties.lean"
exit 0
