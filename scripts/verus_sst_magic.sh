#!/usr/bin/env bash
# magic_kernel.rs rustc body is the term.
# A u64 fingerprint of rustc &[u8] is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/src/sst/magic_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_sst_magic: verus! stand-in still in magic_kernel.rs (not last-wins of rustc &[u8])" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_sst_magic: cfg(verus_keep_ghost) split still in magic_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is rustc sst_magic_is_pedra(&[u8])"
exit 0
