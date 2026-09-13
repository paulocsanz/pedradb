#!/usr/bin/env bash
# Extract production cf_kernel.rs catalog entries.
# Charon --start-from existing entries (missing *_as_is names fail charon).
# Aeneas bottoms on cf_encode_effective / decode_cf_key (lifetime/'a str);
# generated sorry is patched to the production if/slice (same class as scan
# closure / Vote Option::eq — generated Lean, restamped here).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/cf-kernel"
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
SRC="$ROOT/crates/pedradb-core/src/cf_kernel.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::key_in_cf_family' \
    --start-from 'crate::key_in_cf_family_as_is' \
    --start-from 'crate::cf_family_of' \
    --start-from 'crate::cf_encode_effective' \
    --start-from 'crate::encode_cf_key' \
    --start-from 'crate::decode_cf_key' \
    --start-from 'crate::infer_sst_cf' \
    --start-from 'crate::compact_rewrites_sst_cf' \
    --start-from 'crate::compact_rewrites_sst_cf_as_is' \
    --dest-file "$OUT/cf_kernel.llbc" )
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/cf_kernel.llbc"
set -e
python3 - "$OUT/lean/CfKernel.lean" <<'PYEOF'
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
PYEOF
if grep -q 'sorry' "$OUT/lean/CfKernel.lean"; then
  echo "FAIL  CfKernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/pedradb-core/src/cf_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.cf"
echo "ok    extract cf → $OUT"
