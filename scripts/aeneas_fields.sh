#!/usr/bin/env bash
# Extract production fields_kernel.rs catalog entries.
# Charon --start-from all 5 entries. Nested borrows in encode_fields /
# encode_fields_as_is leave sorry; patched to an index loop (same class as
# scan closure / cf encode — generated Lean, restamped here).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/fields-kernel"
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
SRC="$ROOT/crates/montanha-fdb-recipes/src/fields_kernel.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::encode_fields' \
    --start-from 'crate::encode_fields_as_is' \
    --start-from 'crate::field_kept' \
    --start-from 'crate::field_kept_as_is' \
    --start-from 'crate::child_bytes_after' \
    --start-from 'crate::child_bytes_after_as_is' \
    --start-from 'crate::decode_fields' \
    --start-from 'crate::decode_pair_first_nul' \
    --dest-file "$OUT/fields_kernel.llbc" )
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/fields_kernel.llbc"
set -e
python3 - "$OUT/lean/FieldsKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old1 = (
    "def encode_fields\n"
    "  (parts : Slice (Slice Std.U8)) : Result (alloc.vec.Vec Std.U8) := do\n"
    "  sorry\n"
)
new1 = r'''@[rust_loop_body]
def encode_fields_loop.body
  (parts : Slice (Slice Std.U8))
  (iter : core.ops.range.Range Std.Usize)
  (out : alloc.vec.Vec Std.U8) :
  Result (ControlFlow ((core.ops.range.Range Std.Usize) × (alloc.vec.Vec Std.U8))
    (alloc.vec.Vec Std.U8))
  := do
  let (o, iter1) ←
    core.iter.range.IteratorRange.next core.iter.range.StepUsize iter
  match o with
  | none => ok (done out)
  | some i =>
    let p ← Slice.index_usize parts i
    let i2 := Slice.len p
    let r ← core.convert.num.ptr_try_from_impls.TryFromU32Usize.try_from i2
    let n ←
      core.result.Result.expect core.num.error.TryFromIntError.Insts.CoreFmtDebug
        r (toStr "field len fits u32")
    let a ← lift (core.num.U32.to_be_bytes n)
    let s ← lift (Array.to_slice a)
    let out1 ← alloc.vec.Vec.extend_from_slice core.clone.CloneU8 out s
    let out2 ← alloc.vec.Vec.extend_from_slice core.clone.CloneU8 out1 p
    ok (cont (iter1, out2))

@[rust_loop]
def encode_fields_loop
  (parts : Slice (Slice Std.U8))
  (iter : core.ops.range.Range Std.Usize)
  (out : alloc.vec.Vec Std.U8) :
  Result (alloc.vec.Vec Std.U8)
  := do
  loop
    (fun (iter1, out1) => encode_fields_loop.body parts iter1 out1)
    (iter, out)

def encode_fields
  (parts : Slice (Slice Std.U8)) : Result (alloc.vec.Vec Std.U8) := do
  let out := alloc.vec.Vec.new Std.U8
  let n := Slice.len parts
  encode_fields_loop parts { start := 0#usize, «end» := n } out
'''
old2 = (
    "def encode_fields_as_is\n"
    "  (parts : Slice (Slice Std.U8)) : Result (alloc.vec.Vec Std.U8) := do\n"
    "  sorry\n"
)
new2 = r'''@[rust_loop_body]
def encode_fields_as_is_loop.body
  (parts : Slice (Slice Std.U8))
  (iter : core.ops.range.Range Std.Usize)
  (out : alloc.vec.Vec Std.U8) :
  Result (ControlFlow ((core.ops.range.Range Std.Usize) × (alloc.vec.Vec Std.U8))
    (alloc.vec.Vec Std.U8))
  := do
  let (o, iter1) ←
    core.iter.range.IteratorRange.next core.iter.range.StepUsize iter
  match o with
  | none => ok (done out)
  | some i =>
    let p ← Slice.index_usize parts i
    if i > 0#usize
    then
      let out1 ← alloc.vec.Vec.push out 0#u8
      let out2 ← alloc.vec.Vec.extend_from_slice core.clone.CloneU8 out1 p
      ok (cont (iter1, out2))
    else
      let out2 ← alloc.vec.Vec.extend_from_slice core.clone.CloneU8 out p
      ok (cont (iter1, out2))

@[rust_loop]
def encode_fields_as_is_loop
  (parts : Slice (Slice Std.U8))
  (iter : core.ops.range.Range Std.Usize)
  (out : alloc.vec.Vec Std.U8) :
  Result (alloc.vec.Vec Std.U8)
  := do
  loop
    (fun (iter1, out1) => encode_fields_as_is_loop.body parts iter1 out1)
    (iter, out)

def encode_fields_as_is
  (parts : Slice (Slice Std.U8)) : Result (alloc.vec.Vec Std.U8) := do
  let out := alloc.vec.Vec.new Std.U8
  let n := Slice.len parts
  encode_fields_as_is_loop parts { start := 0#usize, «end» := n } out
'''
n = 0
if old1 in src:
    src = src.replace(old1, new1, 1)
    n += 1
if old2 in src:
    src = src.replace(old2, new2, 1)
    n += 1
open(p, "w", encoding="utf-8").write(src)
if n:
    print(f"      patched encode_fields ×{n}")
elif "encode_fields_loop.body" in src:
    print("      encode_fields already patched")
else:
    sys.exit("encode_fields patch target not found")
PYEOF
if grep -q 'sorry' "$OUT/lean/FieldsKernel.lean"; then
  echo "FAIL  FieldsKernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/montanha-fdb-recipes/src/fields_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.fields"
echo "ok    extract fields → $OUT"
