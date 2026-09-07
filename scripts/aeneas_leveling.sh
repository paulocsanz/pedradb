#!/usr/bin/env bash
# Extract production leveling.rs catalog entries.
# RUSTFLAGS=--cfg test so Charon sees #[cfg(test)] as_is mutants.
# Iterator collect/min/max leave holes; patched to index loops (generated
# Lean, restamped here).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/leveling-kernel"
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
SRC="$ROOT/crates/pedradb-core/src/leveling.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  RUSTFLAGS="${RUSTFLAGS:-} --cfg test" "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::level_target_bytes' \
    --start-from 'crate::level_target_bytes_as_is' \
    --start-from 'crate::pick_l0_to_l1' \
    --start-from 'crate::pick_l0_to_l1_as_is_whole_level' \
    --start-from 'crate::pick_pushdown' \
    --start-from 'crate::pick_pushdown_as_is_blind' \
    --start-from 'crate::leveled_enabled' \
    --start-from 'crate::total_bytes' \
    --dest-file "$OUT/leveling_kernel.llbc"
)
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/leveling_kernel.llbc"
set -e
python3 - "$OUT/lean/LevelingKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
n = 0

def repl(old, new, label):
    global src, n
    if old not in src:
        sys.exit(f"patch target not found: {label}")
    src = src.replace(old, new, 1)
    n += 1

repl(
    "def is_disjoint (files : Slice LevelFile) : Result Bool := do\n  sorry\n",
    r'''@[rust_loop_body]
def is_disjoint_inner_loop.body
  (files : Slice LevelFile) (a : LevelFile) (j : Std.Usize) :
  Result (ControlFlow Std.Usize Bool)
  := do
  let n := Slice.len files
  if j < n
  then
    let b ← Slice.index_usize files j
    let sa ← alloc.vec.Vec.as_slice Global a.lo
    let sb ← alloc.vec.Vec.as_slice Global b.lo
    let a_first ←
      Shared1A.Insts.CoreCmpPartialOrdShared0B.le
        (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) sa sb
    let okpair ←
      if a_first
      then
        let ha ← alloc.vec.Vec.as_slice Global a.hi
        Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
          (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) ha sb
      else
        let hb ← alloc.vec.Vec.as_slice Global b.hi
        Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
          (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hb sa
    if okpair
    then
      let j1 ← j + 1#usize
      ok (cont j1)
    else ok (done false)
  else ok (done true)

@[rust_loop]
def is_disjoint_inner_loop
  (files : Slice LevelFile) (a : LevelFile) (j : Std.Usize) :
  Result Bool
  := do
  loop (fun j1 => is_disjoint_inner_loop.body files a j1) j

@[rust_loop_body]
def is_disjoint_outer_loop.body
  (files : Slice LevelFile) (i : Std.Usize) :
  Result (ControlFlow Std.Usize Bool)
  := do
  let n := Slice.len files
  if i < n
  then
    let a ← Slice.index_usize files i
    let j ← i + 1#usize
    let b ← is_disjoint_inner_loop files a j
    if b
    then
      let i1 ← i + 1#usize
      ok (cont i1)
    else ok (done false)
  else ok (done true)

@[rust_loop]
def is_disjoint_outer_loop
  (files : Slice LevelFile) (i : Std.Usize) : Result Bool := do
  loop (fun i1 => is_disjoint_outer_loop.body files i1) i

def is_disjoint (files : Slice LevelFile) : Result Bool := do
  is_disjoint_outer_loop files 0#usize
''',
    "is_disjoint",
)

repl(
    """def
  pick_l0_to_l1.closure_1.Insts.CoreOpsFunctionFnMutTupleSharedSharedLevelFileSharedSliceU8.call_mut
  (c : pick_l0_to_l1.closure_1) (tupled_args : LevelFile) :
  Result ((Slice Std.U8) × pick_l0_to_l1.closure_1)
  := do
  sorry
""",
    """def
  pick_l0_to_l1.closure_1.Insts.CoreOpsFunctionFnMutTupleSharedSharedLevelFileSharedSliceU8.call_mut
  (c : pick_l0_to_l1.closure_1) (tupled_args : LevelFile) :
  Result ((Slice Std.U8) × pick_l0_to_l1.closure_1)
  := do
  let s := alloc.vec.Vec.deref tupled_args.hi
  ok (s, c)
""",
    "pick_l0 closure_1 hi",
)

repl(
    """def
  pick_l0_to_l1.closure.Insts.CoreOpsFunctionFnMutTupleSharedSharedLevelFileSharedSliceU8.call_mut
  (c : pick_l0_to_l1.closure) (tupled_args : LevelFile) :
  Result ((Slice Std.U8) × pick_l0_to_l1.closure)
  := do
  sorry
""",
    """def
  pick_l0_to_l1.closure.Insts.CoreOpsFunctionFnMutTupleSharedSharedLevelFileSharedSliceU8.call_mut
  (c : pick_l0_to_l1.closure) (tupled_args : LevelFile) :
  Result ((Slice Std.U8) × pick_l0_to_l1.closure)
  := do
  let s := alloc.vec.Vec.deref tupled_args.lo
  ok (s, c)
""",
    "pick_l0 closure lo",
)

repl(
    """def pick_l0_to_l1
  (l0 : Slice LevelFile) (l1 : Slice LevelFile) (max_l0 : Std.Usize) :
  Result (Option ((alloc.vec.Vec Std.Usize) × (alloc.vec.Vec Std.Usize)))
  := do
  sorry
""",
    r'''@[rust_loop_body]
def pick_l0_sel_loop.body
  (l0 : Slice LevelFile) (n : Std.Usize)
  (sel : alloc.vec.Vec Std.Usize)
  (hull_lo : alloc.vec.Vec Std.U8) (hull_hi : alloc.vec.Vec Std.U8)
  (i : Std.Usize) :
  Result (ControlFlow
    ((alloc.vec.Vec Std.Usize) × (alloc.vec.Vec Std.U8) ×
      (alloc.vec.Vec Std.U8) × Std.Usize)
    ((alloc.vec.Vec Std.Usize) × (alloc.vec.Vec Std.U8) ×
      (alloc.vec.Vec Std.U8)))
  := do
  if i < n
  then
    let f ← Slice.index_usize l0 i
    let sel1 ← alloc.vec.Vec.push sel f.idx
    let slo ← alloc.vec.Vec.as_slice Global f.lo
    let hlo ← alloc.vec.Vec.as_slice Global hull_lo
    let lo_lt ←
      Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
        (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) slo hlo
    let hull_lo1 ←
      match lo_lt with
      | true => alloc.vec.CloneVec.clone core.clone.CloneU8 f.lo
      | false => ok hull_lo
    let shi ← alloc.vec.Vec.as_slice Global f.hi
    let hhi ← alloc.vec.Vec.as_slice Global hull_hi
    let hi_lt ←
      Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
        (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hhi shi
    let hull_hi1 ←
      match hi_lt with
      | true => alloc.vec.CloneVec.clone core.clone.CloneU8 f.hi
      | false => ok hull_hi
    let i1 ← i + 1#usize
    ok (cont (sel1, hull_lo1, hull_hi1, i1))
  else ok (done (sel, hull_lo, hull_hi))

@[rust_loop]
def pick_l0_sel_loop
  (l0 : Slice LevelFile) (n : Std.Usize)
  (sel : alloc.vec.Vec Std.Usize)
  (hull_lo : alloc.vec.Vec Std.U8) (hull_hi : alloc.vec.Vec Std.U8)
  (i : Std.Usize) :
  Result ((alloc.vec.Vec Std.Usize) × (alloc.vec.Vec Std.U8) ×
    (alloc.vec.Vec Std.U8))
  := do
  loop
    (fun (sel1, hull_lo1, hull_hi1, i1) =>
      pick_l0_sel_loop.body l0 n sel1 hull_lo1 hull_hi1 i1)
    (sel, hull_lo, hull_hi, i)

@[rust_loop_body]
def pick_l0_slice_loop.body
  (l1 : Slice LevelFile)
  (hull_lo : alloc.vec.Vec Std.U8) (hull_hi : alloc.vec.Vec Std.U8)
  (slice : alloc.vec.Vec Std.Usize) (i : Std.Usize) :
  Result (ControlFlow ((alloc.vec.Vec Std.Usize) × Std.Usize)
    (alloc.vec.Vec Std.Usize))
  := do
  let n := Slice.len l1
  if i < n
  then
    let f ← Slice.index_usize l1 i
    let s := alloc.vec.Vec.deref hull_lo
    let s1 := alloc.vec.Vec.deref hull_hi
    let b ← LevelFile.overlaps f s s1
    let slice1 ←
      if b then alloc.vec.Vec.push slice f.idx else ok slice
    let i1 ← i + 1#usize
    ok (cont (slice1, i1))
  else ok (done slice)

@[rust_loop]
def pick_l0_slice_loop
  (l1 : Slice LevelFile)
  (hull_lo : alloc.vec.Vec Std.U8) (hull_hi : alloc.vec.Vec Std.U8)
  (slice : alloc.vec.Vec Std.Usize) (i : Std.Usize) :
  Result (alloc.vec.Vec Std.Usize)
  := do
  loop
    (fun (slice1, i1) =>
      pick_l0_slice_loop.body l1 hull_lo hull_hi slice1 i1)
    (slice, i)

def pick_l0_to_l1
  (l0 : Slice LevelFile) (l1 : Slice LevelFile) (max_l0 : Std.Usize) :
  Result (Option ((alloc.vec.Vec Std.Usize) × (alloc.vec.Vec Std.Usize)))
  := do
  let b ← core.slice.Slice.is_empty l0
  if b
  then ok none
  else
    if max_l0 = 0#usize
    then ok none
    else
      let n0 := Slice.len l0
      let n := if n0 < max_l0 then n0 else max_l0
      let f0 ← Slice.index_usize l0 0#usize
      let hull_lo ← alloc.vec.CloneVec.clone core.clone.CloneU8 f0.lo
      let hull_hi ← alloc.vec.CloneVec.clone core.clone.CloneU8 f0.hi
      let sel0 := alloc.vec.Vec.new Std.Usize
      let sel1 ← alloc.vec.Vec.push sel0 f0.idx
      let (sel, hull_lo1, hull_hi1) ←
        pick_l0_sel_loop l0 n sel1 hull_lo hull_hi 1#usize
      let slice0 := alloc.vec.Vec.new Std.Usize
      let slice ← pick_l0_slice_loop l1 hull_lo1 hull_hi1 slice0 0#usize
      ok (some (sel, slice))
''',
    "pick_l0_to_l1",
)

repl(
    """def pick_pushdown
  (src : Slice LevelFile) (dst : Slice LevelFile) :
  Result (Option (Std.Usize × (alloc.vec.Vec Std.Usize)))
  := do
  sorry
""",
    r'''def pick_pushdown
  (src : Slice LevelFile) (dst : Slice LevelFile) :
  Result (Option (Std.Usize × (alloc.vec.Vec Std.Usize)))
  := do
  let b ← core.slice.Slice.is_empty src
  if b
  then ok none
  else
    let d ← is_disjoint dst
    if d
    then
      let source ← Slice.index_usize src 0#usize
      let slice0 := alloc.vec.Vec.new Std.Usize
      let slice ←
        pick_l0_slice_loop dst source.lo source.hi slice0 0#usize
      ok (some (source.idx, slice))
    else ok none
''',
    "pick_pushdown",
)

repl(
    """def pick_pushdown_as_is_blind
  (src : Slice LevelFile) (dst : Slice LevelFile) :
  Result (Option (Std.Usize × (alloc.vec.Vec Std.Usize)))
  := do
  sorry
""",
    r'''def pick_pushdown_as_is_blind
  (src : Slice LevelFile) (dst : Slice LevelFile) :
  Result (Option (Std.Usize × (alloc.vec.Vec Std.Usize)))
  := do
  let b ← core.slice.Slice.is_empty src
  if b
  then ok none
  else
    let source ← Slice.index_usize src 0#usize
    let slice0 := alloc.vec.Vec.new Std.Usize
    let slice ←
      pick_l0_slice_loop dst source.lo source.hi slice0 0#usize
    ok (some (source.idx, slice))
''',
    "pick_pushdown_as_is_blind",
)

import re

# as_is_whole_level is generated (not a hole) but calls Iterator.map.default,
# which Aeneas.Std does not provide. Same index-loop shape as pick_l0.
m = re.search(
    r"def pick_l0_to_l1_as_is_whole_level\n"
    r"  \(l0 : Slice LevelFile\) \(l1 : Slice LevelFile\) :\n"
    r"  Result \(Option \(\(alloc\.vec\.Vec Std\.Usize\) × \(alloc\.vec\.Vec Std\.Usize\)\)\)\n"
    r"  := do\n"
    r"([\s\S]*?)\n"
    r"(/\-\- \[pedra_aeneas_leveling_kernel::pick_pushdown)",
    src,
)
if not m:
    sys.exit("patch target not found: pick_l0_to_l1_as_is_whole_level")
src = (
    src[: m.start()]
    + r'''@[rust_loop_body]
def pick_idx_loop.body
  (files : Slice LevelFile) (out : alloc.vec.Vec Std.Usize) (i : Std.Usize) :
  Result (ControlFlow ((alloc.vec.Vec Std.Usize) × Std.Usize)
    (alloc.vec.Vec Std.Usize))
  := do
  let n := Slice.len files
  if i < n
  then
    let f ← Slice.index_usize files i
    let out1 ← alloc.vec.Vec.push out f.idx
    let i1 ← i + 1#usize
    ok (cont (out1, i1))
  else ok (done out)

@[rust_loop]
def pick_idx_loop
  (files : Slice LevelFile) (out : alloc.vec.Vec Std.Usize) (i : Std.Usize) :
  Result (alloc.vec.Vec Std.Usize)
  := do
  loop (fun (out1, i1) => pick_idx_loop.body files out1 i1) (out, i)

def pick_l0_to_l1_as_is_whole_level
  (l0 : Slice LevelFile) (l1 : Slice LevelFile) :
  Result (Option ((alloc.vec.Vec Std.Usize) × (alloc.vec.Vec Std.Usize)))
  := do
  let b ← core.slice.Slice.is_empty l0
  if b
  then ok none
  else
    let v0 := alloc.vec.Vec.new Std.Usize
    let v ← pick_idx_loop l0 v0 0#usize
    let v10 := alloc.vec.Vec.new Std.Usize
    let v1 ← pick_idx_loop l1 v10 0#usize
    ok (some (v, v1))

'''
    + m.group(2)
    + src[m.end() :]
)
n += 1

# Aeneas.Std Iterator is next + step_by/enumerate/take (defaults). Generated
# impls fill map/filter/collect/all/max/min which are not fields.
src, n_iter = re.subn(
    r"\n  map := fun[\s\S]*?\n  take :=",
    "\n  take :=",
    src,
)
src, n_coll = re.subn(
    r"(take :=[^\n]+(?:\n    [^\n]+)*)\n  (?:collect|all|max|min) := fun[\s\S]*?\n\}",
    r"\1\n}",
    src,
)
open(p, "w", encoding="utf-8").write(src)
print(f"      patched leveling ×{n} (Iterator fields -{n_iter}/-{n_coll})")
PYEOF
if grep -q 'sorry' "$OUT/lean/LevelingKernel.lean"; then
  echo "FAIL  LevelingKernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/pedradb-core/src/leveling.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.leveling"
echo "ok    extract leveling → $OUT"
