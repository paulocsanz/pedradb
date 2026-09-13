#!/usr/bin/env bash
# Extract production probe_order_kernel.rs catalog entries (first_probe_on_equal_lo).
# Charon --start-from: probe_order walk is Iterator-refused (whole-file CFailure).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/probe-order-kernel"
OUT="$ROOT/formal/aeneas/out"
REQUIRED=0
if [[ "${1:-}" == "--required" ]]; then
  REQUIRED=1
fi

CHARON="${CHARON:-$(command -v charon || true)}"
AENEAS="${AENEAS:-$(command -v aeneas || true)}"
if [[ -z "$CHARON" || -z "$AENEAS" ]]; then
  msg="charon/aeneas not on PATH. See formal/aeneas/PINS.md"
  if [[ "$REQUIRED" -eq 1 ]]; then
    echo "FAIL  $msg" >&2
    exit 1
  fi
  echo "skip  $msg"
  exit 0
fi

mkdir -p "$OUT"
SRC="$ROOT/crates/pedradb-core/src/probe_order_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::first_probe_on_equal_lo' \
    --start-from 'crate::first_probe_on_equal_lo_as_is' \
    --start-from 'crate::run_pairwise_disjoint_los' \
    --start-from 'crate::run_pairwise_disjoint_los_as_is' \
    --start-from 'crate::probe_order_covering' \
    --start-from 'crate::probe_order_covering_as_is' \
    --dest-file "$OUT/probe_order_kernel.llbc"
)
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/probe_order_kernel.llbc"
set -e
python3 - "$OUT/lean/ProbeOrderKernel.lean" <<'PYEOF'
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
    """def
  run_pairwise_disjoint_los.closure.Insts.CoreOpsFunctionFnMutTupleUsizeBool.call_mut
  (c : run_pairwise_disjoint_los.closure) (tupled_args : Std.Usize) :
  Result (Bool × run_pairwise_disjoint_los.closure)
  := do
  sorry
""",
    """def
  run_pairwise_disjoint_los.closure.Insts.CoreOpsFunctionFnMutTupleUsizeBool.call_mut
  (c : run_pairwise_disjoint_los.closure) (tupled_args : Std.Usize) :
  Result (Bool × run_pairwise_disjoint_los.closure)
  := do
  let (his, los) := c
  let i1 ← tupled_args - 1#usize
  let hi ← Slice.index_usize his i1
  let lo ← Slice.index_usize los tupled_args
  let b ←
    Slice.Insts.CoreCmpPartialOrdSlice.lt core.cmp.PartialOrdU8 hi lo
  ok (b, c)
""",
    "disjoint call_mut",
)
repl(
    """def
  run_pairwise_disjoint_los_as_is.closure.Insts.CoreOpsFunctionFnMutTupleUsizeBool.call_mut
  (c : run_pairwise_disjoint_los_as_is.closure) (tupled_args : Std.Usize) :
  Result (Bool × run_pairwise_disjoint_los_as_is.closure)
  := do
  sorry
""",
    """def
  run_pairwise_disjoint_los_as_is.closure.Insts.CoreOpsFunctionFnMutTupleUsizeBool.call_mut
  (c : run_pairwise_disjoint_los_as_is.closure) (tupled_args : Std.Usize) :
  Result (Bool × run_pairwise_disjoint_los_as_is.closure)
  := do
  let (his, los) := c
  let i1 ← tupled_args - 1#usize
  let hi ← Slice.index_usize his i1
  let lo ← Slice.index_usize los tupled_args
  let b ←
    Slice.Insts.CoreCmpPartialOrdSlice.le core.cmp.PartialOrdU8 hi lo
  ok (b, c)
""",
    "disjoint as_is call_mut",
)
repl(
    """def probe_order_covering
  (newest_first : Slice Std.Usize) (by_lo : Slice Std.Usize)
  (prefix_end : Std.Usize) (his : Slice (Slice Std.U8)) (key : Slice Std.U8) :
  Result (alloc.vec.Vec Std.Usize)
  := do
  sorry
""",
    """@[rust_loop_body]
def probe_order_covering_loop.body
  (newest_first : Slice Std.Usize) (by_lo : Slice Std.Usize)
  (prefix_end : Std.Usize) (his : Slice (Slice Std.U8)) (key : Slice Std.U8)
  (out : alloc.vec.Vec Std.Usize) (k : Std.Usize) :
  Result (ControlFlow ((alloc.vec.Vec Std.Usize) × Std.Usize) (alloc.vec.Vec Std.Usize))
  := do
  let n := Slice.len newest_first
  if k < n
  then
    let i ← Slice.index_usize newest_first k
    let pos ← covering_pos by_lo i
    let nlo := Slice.len by_lo
    if nlo <= pos
    then
      let out1 ← alloc.vec.Vec.push out i
      let k1 ← k + 1#usize
      ok (cont (out1, k1))
    else if pos < prefix_end
    then
      let b ← covering_hi_ge his pos key
      if b
      then
        let out1 ← alloc.vec.Vec.push out i
        let k1 ← k + 1#usize
        ok (cont (out1, k1))
      else
        let k1 ← k + 1#usize
        ok (cont (out, k1))
    else
      let k1 ← k + 1#usize
      ok (cont (out, k1))
  else ok (done out)

@[rust_loop]
def probe_order_covering_loop
  (newest_first : Slice Std.Usize) (by_lo : Slice Std.Usize)
  (prefix_end : Std.Usize) (his : Slice (Slice Std.U8)) (key : Slice Std.U8)
  (out : alloc.vec.Vec Std.Usize) (k : Std.Usize) :
  Result (alloc.vec.Vec Std.Usize)
  := do
  loop
    (fun (out1, k1) =>
      probe_order_covering_loop.body newest_first by_lo prefix_end his key out1 k1)
    (out, k)

def probe_order_covering
  (newest_first : Slice Std.Usize) (by_lo : Slice Std.Usize)
  (prefix_end : Std.Usize) (his : Slice (Slice Std.U8)) (key : Slice Std.U8) :
  Result (alloc.vec.Vec Std.Usize)
  := do
  let n := Slice.len newest_first
  let out := alloc.vec.Vec.with_capacity Std.Usize n
  probe_order_covering_loop newest_first by_lo prefix_end his key out 0#usize
""",
    "covering loop",
)
repl(
    """def probe_order_covering_as_is
  (newest_first : Slice Std.Usize) (by_lo : Slice Std.Usize)
  (prefix_end : Std.Usize) (his : Slice (Slice Std.U8)) (key : Slice Std.U8) :
  Result (alloc.vec.Vec Std.Usize)
  := do
  sorry
""",
    """@[rust_loop_body]
def probe_order_covering_as_is_loop.body
  (newest_first : Slice Std.Usize) (by_lo : Slice Std.Usize)
  (prefix_end : Std.Usize) (his : Slice (Slice Std.U8)) (key : Slice Std.U8)
  (out : alloc.vec.Vec Std.Usize) (j : Std.Usize) :
  Result (ControlFlow ((alloc.vec.Vec Std.Usize) × Std.Usize) (alloc.vec.Vec Std.Usize))
  := do
  let n := Slice.len newest_first
  if j < n
  then
    let last ← n - 1#usize
    let idx ← last - j
    let i ← Slice.index_usize newest_first idx
    let pos ← covering_pos by_lo i
    let nlo := Slice.len by_lo
    if nlo <= pos
    then
      let out1 ← alloc.vec.Vec.push out i
      let j1 ← j + 1#usize
      ok (cont (out1, j1))
    else if pos < prefix_end
    then
      let b ← covering_hi_ge his pos key
      if b
      then
        let out1 ← alloc.vec.Vec.push out i
        let j1 ← j + 1#usize
        ok (cont (out1, j1))
      else
        let j1 ← j + 1#usize
        ok (cont (out, j1))
    else
      let j1 ← j + 1#usize
      ok (cont (out, j1))
  else ok (done out)

@[rust_loop]
def probe_order_covering_as_is_loop
  (newest_first : Slice Std.Usize) (by_lo : Slice Std.Usize)
  (prefix_end : Std.Usize) (his : Slice (Slice Std.U8)) (key : Slice Std.U8)
  (out : alloc.vec.Vec Std.Usize) (j : Std.Usize) :
  Result (alloc.vec.Vec Std.Usize)
  := do
  loop
    (fun (out1, j1) =>
      probe_order_covering_as_is_loop.body newest_first by_lo prefix_end his key
        out1 j1)
    (out, j)

def probe_order_covering_as_is
  (newest_first : Slice Std.Usize) (by_lo : Slice Std.Usize)
  (prefix_end : Std.Usize) (his : Slice (Slice Std.U8)) (key : Slice Std.U8) :
  Result (alloc.vec.Vec Std.Usize)
  := do
  let n := Slice.len newest_first
  let out := alloc.vec.Vec.with_capacity Std.Usize n
  probe_order_covering_as_is_loop newest_first by_lo prefix_end his key out
    0#usize
""",
    "covering as_is loop",
)
if "sorry" in src:
    sys.exit("ProbeOrderKernel.lean still contains sorry")
open(p, "w", encoding="utf-8").write(src)
print(f"      patched probe_order ×{n}")
PYEOF
if grep -q 'sorry' "$OUT/lean/ProbeOrderKernel.lean"; then
  echo "FAIL  ProbeOrderKernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/pedradb-core/src/probe_order_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.probe_order"
echo "ok    extract probe_order → $OUT"
