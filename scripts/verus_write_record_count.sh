#!/usr/bin/env bash
# batch.rs rustc body is the term (Aeneas Batch.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/src/batch.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_write_record_count: verus! stand-in still in batch.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_write_record_count: cfg(verus_keep_ghost) split still in batch.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Batch.lean"
exit 0
