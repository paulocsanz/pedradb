# Post-extract patch for kernel 'fields' — moved verbatim from
# scripts/aeneas_fields.sh. Applied by `cargo xtask proof fields` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/FieldsKernel.lean']
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
