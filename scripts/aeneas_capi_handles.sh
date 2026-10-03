#!/usr/bin/env bash
# Extract production capi handles.rs catalog entries (c_len_admitted)
# plus RFC-0075 path-walk / free-table gates (same --start-from; rest of
# handles.rs is IterMut).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/capi-handles-kernel"
OUT="$ROOT/formal/aeneas/out"
REQUIRED=0
if [[ "${1:-}" == "--required" ]]; then
  REQUIRED=1
fi
CHARON="${CHARON:-$(command -v charon || true)}"
AENEAS="${AENEAS:-$(command -v aeneas || true)}"
if [[ -z "$CHARON" || -z "$AENEAS" ]]; then
  msg="charon/aeneas not on PATH. See formal/aeneas/PINS.md"
  if [[ "$REQUIRED" -eq 1 ]]; then echo "FAIL  $msg" >&2; exit 1; fi
  echo "skip  $msg"; exit 0
fi
mkdir -p "$OUT"
SRC="$ROOT/crates/pedradb-capi/src/handles_kernel.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::c_len_admitted' \
    --start-from 'crate::c_len_admitted_as_is' \
    --start-from 'crate::c_path_walk_bytes' \
    --start-from 'crate::c_path_walk_bytes_as_is' \
    --start-from 'crate::c_path_nul_off_admitted' \
    --start-from 'crate::c_path_nul_off_admitted_as_is' \
    --start-from 'crate::c_free_table_admitted' \
    --start-from 'crate::c_free_table_admitted_as_is' \
    --dest-file "$OUT/capi_handles_kernel.llbc" )
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/capi_handles_kernel.llbc"
{
  echo "path=crates/pedradb-capi/src/handles_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.capi_handles"
echo "ok    extract capi_handles → $OUT"
