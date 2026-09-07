#!/usr/bin/env bash
# Extract production world_kernel.rs catalog entries.
# Aeneas bottoms on Option<&'static str> and HashMap fold; patched
# (generated Lean, restamped here).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/world-kernel"
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
SRC="$ROOT/crates/pedradb-world/src/world_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas \
    --start-from-if-exists 'crate::trajectory_violation' \
    --start-from-if-exists 'crate::trajectory_violation_as_is' \
    --start-from-if-exists 'crate::check_trajectory' \
    --start-from-if-exists 'crate::check_trajectory_as_is' \
    --dest-file "$OUT/world_kernel.llbc"
)
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/world_kernel.llbc"
set -e
python3 - "$OUT/lean/WorldKernel.lean" <<'PYEOF'
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
    """def trajectory_violation
  (prev : TrajectorySample) (cur : TrajectorySample) :
  Result (Option Str)
  := do
  sorry
""",
    r'''def trajectory_violation
  (prev : TrajectorySample) (cur : TrajectorySample) :
  Result (Option Str)
  := do
  if cur.term < prev.term
  then ok (some (toStr "term"))
  else
    if cur.snapshot_index < prev.snapshot_index
    then ok (some (toStr "snapshot_index"))
    else
      if cur.applied_index < prev.applied_index
      then ok (some (toStr "applied_index"))
      else ok none
''',
    "trajectory_violation",
)
repl(
    """def trajectory_violation_as_is
  (prev : TrajectorySample) (cur : TrajectorySample) :
  Result (Option Str)
  := do
  sorry
""",
    r'''def trajectory_violation_as_is
  (prev : TrajectorySample) (cur : TrajectorySample) :
  Result (Option Str)
  := do
  if cur.term < prev.term
  then ok (some (toStr "term"))
  else ok none
''',
    "trajectory_violation_as_is",
)
repl(
    """def check_trajectory
  (samples : Slice TrajectorySample) : Result (alloc.vec.Vec String) := do
  sorry
""",
    r'''axiom str_to_string : Str → Result String

@[rust_loop_body]
def find_prev_loop.body
  (samples : Slice TrajectorySample) (s : TrajectorySample) (j : Std.Usize) :
  Result (ControlFlow Std.Usize (Option TrajectorySample))
  := do
  if j > 0#usize
  then
    let j1 ← j - 1#usize
    let p ← Slice.index_usize samples j1
    if p.node = s.node
    then
      if p.range = s.range
      then ok (done (some p))
      else ok (cont j1)
    else ok (cont j1)
  else ok (done none)

@[rust_loop]
def find_prev_loop
  (samples : Slice TrajectorySample) (s : TrajectorySample) (j : Std.Usize) :
  Result (Option TrajectorySample)
  := do
  loop (fun j1 => find_prev_loop.body samples s j1) j

@[rust_loop_body]
def check_trajectory_loop.body
  (samples : Slice TrajectorySample) (out : alloc.vec.Vec String)
  (i : Std.Usize) :
  Result (ControlFlow ((alloc.vec.Vec String) × Std.Usize)
    (alloc.vec.Vec String))
  := do
  let n := Slice.len samples
  if i < n
  then
    let s ← Slice.index_usize samples i
    let o ← find_prev_loop samples s i
    let out1 ←
      match o with
      | none => ok out
      | some p =>
        let w ← trajectory_violation p s
        match w with
        | none => ok out
        | some what =>
          let msg ← str_to_string what
          alloc.vec.Vec.push out msg
    let i1 ← i + 1#usize
    ok (cont (out1, i1))
  else ok (done out)

@[rust_loop]
def check_trajectory_loop
  (samples : Slice TrajectorySample) (out : alloc.vec.Vec String)
  (i : Std.Usize) : Result (alloc.vec.Vec String)
  := do
  loop
    (fun (out1, i1) => check_trajectory_loop.body samples out1 i1)
    (out, i)

def check_trajectory
  (samples : Slice TrajectorySample) : Result (alloc.vec.Vec String) := do
  let out0 := alloc.vec.Vec.new String
  check_trajectory_loop samples out0 0#usize
''',
    "check_trajectory",
)
repl(
    """def check_trajectory_as_is
  (samples : Slice TrajectorySample) : Result (alloc.vec.Vec String) := do
  sorry
""",
    r'''@[rust_loop_body]
def check_trajectory_as_is_loop.body
  (samples : Slice TrajectorySample) (out : alloc.vec.Vec String)
  (i : Std.Usize) :
  Result (ControlFlow ((alloc.vec.Vec String) × Std.Usize)
    (alloc.vec.Vec String))
  := do
  let n := Slice.len samples
  if i < n
  then
    let s ← Slice.index_usize samples i
    let o ← find_prev_loop samples s i
    let out1 ←
      match o with
      | none => ok out
      | some p =>
        let w ← trajectory_violation_as_is p s
        match w with
        | none => ok out
        | some what =>
          let msg ← str_to_string what
          alloc.vec.Vec.push out msg
    let i1 ← i + 1#usize
    ok (cont (out1, i1))
  else ok (done out)

@[rust_loop]
def check_trajectory_as_is_loop
  (samples : Slice TrajectorySample) (out : alloc.vec.Vec String)
  (i : Std.Usize) : Result (alloc.vec.Vec String)
  := do
  loop
    (fun (out1, i1) => check_trajectory_as_is_loop.body samples out1 i1)
    (out, i)

def check_trajectory_as_is
  (samples : Slice TrajectorySample) : Result (alloc.vec.Vec String) := do
  let out0 := alloc.vec.Vec.new String
  check_trajectory_as_is_loop samples out0 0#usize
''',
    "check_trajectory_as_is",
)

if "sorry" in src:
    sys.exit("WorldKernel.lean still contains sorry")
open(p, "w", encoding="utf-8").write(src)
print(f"      patched world ×{n}")
PYEOF
if grep -q 'sorry' "$OUT/lean/WorldKernel.lean"; then
  echo "FAIL  WorldKernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/pedradb-world/src/world_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.world"
echo "ok    extract world → $OUT"
