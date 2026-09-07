#!/usr/bin/env bash
# Extract production lsm_r1_kernel.rs catalog entries.
# Charon --start-from probe/compact/reopen/r1_modelo. Nested-loop returns
# make compact an axiom and reopen_as_is a hole; patched to index loops
# over level_put/level_remove (generated Lean, restamped here).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/lsm-r1-kernel"
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
SRC="$ROOT/crates/pedradb-core/src/lsm_r1_kernel.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::lsm_probe' \
    --start-from 'crate::lsm_probe_as_is' \
    --start-from 'crate::lsm_compact' \
    --start-from 'crate::lsm_compact_as_is' \
    --start-from 'crate::lsm_reopen' \
    --start-from 'crate::lsm_reopen_as_is' \
    --start-from 'crate::r1_modelo' \
    --start-from 'crate::r1_modelo_as_is' \
    --dest-file "$OUT/lsm_r1_kernel.llbc" )
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/lsm_r1_kernel.llbc"
set -e
python3 - "$OUT/lean/LsmR1Kernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = """axiom lsm_compact : LsmState → Std.Usize → Result (Option LsmState)

/-- [pedra_aeneas_lsm_r1_kernel::lsm_compact_as_is]:
    Source: '../../../crates/pedradb-core/src/lsm_r1_kernel.rs', lines 264:0-290:1
    Visibility: public -/
axiom lsm_compact_as_is : LsmState → Std.Usize → Result (Option LsmState)

/-- [pedra_aeneas_lsm_r1_kernel::lsm_reopen]:
    Source: '../../../crates/pedradb-core/src/lsm_r1_kernel.rs', lines 294:0-296:1
    Visibility: public -/
def lsm_reopen (s : LsmState) : Result LsmState := do
  ok s

/-- [pedra_aeneas_lsm_r1_kernel::lsm_reopen_as_is]:
    Source: '../../../crates/pedradb-core/src/lsm_r1_kernel.rs', lines 300:0-308:1
    Visibility: public -/
def lsm_reopen_as_is (s : LsmState) : Result LsmState := do
  sorry
"""
new = r'''@[rust_loop_body]
def lsm_compact_inner_loop.body
  (drop_all_tombs : Bool) (depth : Std.Usize) (src : LsmLevel)
  (out : LsmState) (i : Std.Usize) :
  Result (ControlFlow (LsmState × Std.Usize) (Option LsmState))
  := do
  if i < src.len
  then
    let e ← Array.index_usize src.entries i
    let dst ← Array.index_usize out.levels depth
    let max1 ← MAX_LEVELS - 1#usize
    if e.tomb && (drop_all_tombs || depth = max1)
    then
      let dst1 ← level_remove dst e.key
      let levels1 ← Array.update out.levels depth dst1
      let i1 ← i + 1#usize
      ok (cont ({ levels := levels1, next_seq := out.next_seq }, i1))
    else
      let (ok1, dst1) ← level_put dst e
      if ok1
      then
        let levels1 ← Array.update out.levels depth dst1
        let i1 ← i + 1#usize
        ok (cont ({ levels := levels1, next_seq := out.next_seq }, i1))
      else ok (done none)
  else ok (done (some out))

@[rust_loop]
def lsm_compact_inner_loop
  (drop_all_tombs : Bool) (depth : Std.Usize) (src : LsmLevel)
  (out : LsmState) (i : Std.Usize) :
  Result (Option LsmState)
  := do
  loop
    (fun (out1, i1) =>
      lsm_compact_inner_loop.body drop_all_tombs depth src out1 i1)
    (out, i)

@[rust_loop_body]
def lsm_compact_src_loop.body
  (drop_all_tombs : Bool) (depth : Std.Usize)
  (out : LsmState) (src_lvl : Std.Usize) :
  Result (ControlFlow (LsmState × Std.Usize) (Option LsmState))
  := do
  if src_lvl > 0#usize
  then
    let src_lvl1 ← src_lvl - 1#usize
    let src ← Array.index_usize out.levels src_lvl1
    let empty ← LsmLevel.empty
    let levels1 ← Array.update out.levels src_lvl1 empty
    let out1 := { levels := levels1, next_seq := out.next_seq }
    let o ← lsm_compact_inner_loop drop_all_tombs depth src out1 0#usize
    match o with
    | none => ok (done none)
    | some out2 => ok (cont (out2, src_lvl1))
  else ok (done (some out))

@[rust_loop]
def lsm_compact_src_loop
  (drop_all_tombs : Bool) (depth : Std.Usize)
  (out : LsmState) (src_lvl : Std.Usize) :
  Result (Option LsmState)
  := do
  loop
    (fun (out1, src_lvl1) =>
      lsm_compact_src_loop.body drop_all_tombs depth out1 src_lvl1)
    (out, src_lvl)

def lsm_compact
  (s : LsmState) (depth : Std.Usize) : Result (Option LsmState) := do
  if depth = 0#usize
  then ok none
  else
    if depth < MAX_LEVELS
    then lsm_compact_src_loop false depth s depth
    else ok none

def lsm_compact_as_is
  (s : LsmState) (depth : Std.Usize) : Result (Option LsmState) := do
  if depth = 0#usize
  then ok none
  else
    if depth < MAX_LEVELS
    then lsm_compact_src_loop true depth s depth
    else ok none

def lsm_reopen (s : LsmState) : Result LsmState := do
  ok s

@[rust_loop_body]
def lsm_reopen_as_is_loop.body
  (s : LsmState) (levels : Array LsmLevel 4#usize) (i : Std.Usize) :
  Result (ControlFlow ((Array LsmLevel 4#usize) × Std.Usize)
    (Array LsmLevel 4#usize))
  := do
  if i < MAX_LEVELS
  then
    let j ← MAX_LEVELS - 1#usize
    let j1 ← j - i
    let ll ← Array.index_usize s.levels j1
    let a ← Array.update levels i ll
    let i1 ← i + 1#usize
    ok (cont (a, i1))
  else ok (done levels)

@[rust_loop]
def lsm_reopen_as_is_loop
  (s : LsmState) (levels : Array LsmLevel 4#usize) (i : Std.Usize) :
  Result (Array LsmLevel 4#usize)
  := do
  loop
    (fun (levels1, i1) => lsm_reopen_as_is_loop.body s levels1 i1)
    (levels, i)

def lsm_reopen_as_is (s : LsmState) : Result LsmState := do
  let a ← lsm_reopen_as_is_loop s s.levels 0#usize
  ok { levels := a, next_seq := s.next_seq }
'''
if old in src:
    src = src.replace(old, new, 1)
    open(p, "w", encoding="utf-8").write(src)
    print("      patched lsm_compact / lsm_reopen_as_is")
elif "def lsm_compact_inner_loop.body" in src:
    print("      lsm_compact already patched")
else:
    sys.exit("lsm_compact patch target not found")
PYEOF
if grep -q 'sorry' "$OUT/lean/LsmR1Kernel.lean"; then
  echo "FAIL  LsmR1Kernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/pedradb-core/src/lsm_r1_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.lsm_r1"
echo "ok    extract lsm_r1 → $OUT"
