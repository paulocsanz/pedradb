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
    --start-from 'crate::merge::write_op_covers_key' \
    --start-from 'crate::merge::write_op_covers_key_as_is' \
    --start-from 'crate::merge::bound_as_ref' \
    --start-from 'crate::merge::bound_to_owned' \
    --dest-file "$OUT/merge_kernel.llbc" )
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/merge_kernel.llbc"
# Lean 4 `do` match rejects dotted constructors (`key.ValueType.Deletion`)
# as pattern variables. Same semantics: drop `do`, use `.Deletion`.
python3 - "$OUT/lean/MergeKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = (
    "  Result Bool\n"
    "  := do\n"
    "  match kind with\n"
    "  | key.ValueType.Deletion =>\n"
    "    core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 start key\n"
    "  | key.ValueType.Value =>\n"
    "    core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 start key\n"
    "  | key.ValueType.RangeDeletion => merge.range_tombstone_covers start end1 key\n"
)
new = (
    "  Result Bool\n"
    "  :=\n"
    "  match kind with\n"
    "  | .Deletion =>\n"
    "    core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 start key\n"
    "  | .Value =>\n"
    "    core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 start key\n"
    "  | .RangeDeletion => merge.range_tombstone_covers start end1 key\n"
)
old2 = (
    "  Result Bool\n"
    "  := do\n"
    "  match kind with\n"
    "  | key.ValueType.Deletion => ok false\n"
    "  | key.ValueType.Value => ok false\n"
    "  | key.ValueType.RangeDeletion =>\n"
    "    merge.range_tombstone_covers_as_is start end1 key\n"
)
new2 = (
    "  Result Bool\n"
    "  :=\n"
    "  match kind with\n"
    "  | .Deletion => ok false\n"
    "  | .Value => ok false\n"
    "  | .RangeDeletion =>\n"
    "    merge.range_tombstone_covers_as_is start end1 key\n"
)
n = 0
if old in src:
    src = src.replace(old, new, 1)
    n += 1
elif new not in src:
    sys.exit("write_op_covers_key match patch target not found")
if old2 in src:
    src = src.replace(old2, new2, 1)
    n += 1
elif new2 not in src:
    sys.exit("write_op_covers_key_as_is match patch target not found")
open(p, "w", encoding="utf-8").write(src)
print(f"      patched write_op_covers_key do-match ({n})")
PYEOF
{
  echo "path=crates/pedradb-core/src/merge.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.merge"
echo "ok    extract merge → $OUT"
