#!/usr/bin/env bash
# manifest_kernel.rs rustc body is the term (Aeneas Manifest.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/src/manifest_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_manifest_recover: verus! stand-in still in manifest_kernel.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_manifest_recover: cfg(verus_keep_ghost) split still in manifest_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Manifest.lean"
exit 0
