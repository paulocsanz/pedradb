# Post-extract patch for kernel 'cf' — moved verbatim from
# scripts/aeneas_cf.sh. Applied by `cargo xtask proof cf` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/CfKernel.lean']
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
n = 0
old1 = (
    "def cf_encode_effective (cf : Str) (default_raw : Bool) : Result Str := do\n"
    "  sorry\n"
)
new1 = (
    "def cf_encode_effective (cf : Str) (default_raw : Bool) : Result Str := do\n"
    "  let b ← Str.Insts.CoreCmpPartialEqStr.eq cf (toStr \"default\")\n"
    "  if b && default_raw\n"
    "  then ok (toStr \"\")\n"
    "  else ok cf\n"
)
if old1 in src:
    src = src.replace(old1, new1, 1)
    n += 1
old2 = (
    "def decode_cf_key\n"
    "  (cf : Str) (encoded : Slice Std.U8) (default_raw : Bool) :\n"
    "  Result (Slice Std.U8)\n"
    "  := do\n"
    "  sorry\n"
)
new2 = (
    "def decode_cf_key\n"
    "  (cf : Str) (encoded : Slice Std.U8) (default_raw : Bool) :\n"
    "  Result (Slice Std.U8)\n"
    "  := do\n"
    "  let effective ← cf_encode_effective cf default_raw\n"
    "  let b ← core.str.Str.is_empty effective\n"
    "  if b\n"
    "  then ok encoded\n"
    "  else\n"
    "    let i ← core.str.Str.len effective\n"
    "    let i1 ← i + 1#usize\n"
    "    let n := Slice.len encoded\n"
    "    if i1 > n\n"
    "    then lift (Array.to_slice (Std.Array.empty Std.U8))\n"
    "    else\n"
    "      core.slice.index.Slice.index\n"
    "        (core.slice.index.SliceIndexRangeFromUsizeSlice Std.U8) encoded\n"
    "        { start := i1 }\n"
)
if old2 in src:
    src = src.replace(old2, new2, 1)
    n += 1
open(p, "w", encoding="utf-8").write(src)
if n:
    print(f"      patched cf_encode_effective/decode_cf_key ×{n}")
elif "if b && default_raw" in src:
    print("      cf_encode_effective already patched")
else:
    sys.exit("cf encode/decode patch target not found")
