#!/usr/bin/env bash
# ship_kernel.rs rustc body is the term (Aeneas Ship.lean).
# A u64 fingerprint of rustc &[u8] stamps is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-replicate/src/ship_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_ship_stamp: verus! stand-in still in ship_kernel.rs (not last-wins of rustc &[u8])" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_ship_stamp: cfg(verus_keep_ghost) split still in ship_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Ship.lean"
exit 0
