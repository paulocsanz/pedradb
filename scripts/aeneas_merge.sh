#!/usr/bin/env bash
# Extract production merge.rs catalog entries (visible_at) plus
# user_key_in_range / past_end. WindowKvIter / StreamingVisibleIter stay
# Iterator-refused.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/merge-kernel"
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
SRC="$ROOT/crates/pedradb-core/src/merge.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::merge::visible_at' \
    --start-from 'crate::merge::visible_at_as_is' \
    --start-from 'crate::merge::range_tombstone_covers' \
    --start-from 'crate::merge::range_tombstone_covers_as_is' \
    --start-from 'crate::merge::user_key_in_range' \
    --start-from 'crate::merge::past_end' \
    --start-from 'crate::merge::iter_window_keep' \
    --start-from 'crate::merge::iter_window_keep_as_is' \
    --dest-file "$OUT/merge_kernel.llbc" )
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/merge_kernel.llbc"
{
  echo "path=crates/pedradb-core/src/merge.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.merge"
echo "ok    extract merge → $OUT"
