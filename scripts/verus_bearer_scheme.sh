#!/usr/bin/env bash
# auth_kernel.rs rustc body is the term (Aeneas Auth.lean).
# A u8-fold view of rustc &str bearer is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-http/src/auth_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_bearer_scheme: verus! stand-in still in auth_kernel.rs (not last-wins of rustc &str)" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_bearer_scheme: cfg(verus_keep_ghost) split still in auth_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Auth.lean"
exit 0
