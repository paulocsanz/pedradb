# Post-extract patch for kernel 'merge' — moved verbatim from
# scripts/aeneas_merge.sh. Applied by `cargo xtask proof merge` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/MergeKernel.lean']
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
