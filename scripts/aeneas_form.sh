#!/usr/bin/env bash
# Extract production form_kernel.rs catalog entries.
# Charon --exclude of str::contains / pattern (CFailure pattern.rs:99).
# contains Pattern hole and query_values_conflict Iterator.any patched
# (generated Lean, restamped here).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/form-kernel"
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
SRC="$ROOT/crates/pedradb-http/src/form_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas \
    --start-from-if-exists 'crate::form_decode' \
    --start-from-if-exists 'crate::form_decode_as_is' \
    --start-from-if-exists 'crate::form_plus_byte' \
    --start-from-if-exists 'crate::form_plus_byte_as_is' \
    --start-from-if-exists 'crate::from_hex' \
    --start-from-if-exists 'crate::plus_before_percent' \
    --start-from-if-exists 'crate::query_values_conflict' \
    --start-from-if-exists 'crate::query_values_conflict_as_is' \
    --start-from-if-exists 'crate::query_u64_conflict' \
    --start-from-if-exists 'crate::query_u64_conflict_as_is' \
    --start-from-if-exists 'crate::query_part_is_bare_name' \
    --start-from-if-exists 'crate::query_part_is_bare_name_as_is' \
    --exclude 'core::str::{str}::contains' \
    --exclude 'core::str::pattern' \
    --exclude 'core::str::pattern::Pattern' \
    --dest-file "$OUT/form_kernel.llbc"
)
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/form_kernel.llbc"
set -e
python3 - "$OUT/lean/FormKernel.lean" <<'PYEOF'
import re
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

src2, n_ax = re.subn(
    r"axiom core\.str\.Str\.contains\n"
    r"  \{P : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  Str → P → Result Bool\n",
    "axiom core.str.Str.contains : Str → Char → Result Bool\n",
    src,
    count=1,
)
if n_ax != 1:
    sys.exit("patch target not found: contains axiom")
src = src2
n += 1

src2, n_call = re.subn(
    r"core\.str\.Str\.contains sorry /- Could not find: trait_impl_id: \d+-/ part\n"
    r"        '='\n",
    "core.str.Str.contains part '='\n",
    src,
    count=1,
)
if n_call != 1:
    sys.exit("patch target not found: contains call")
src = src2
n += 1

repl(
    "def query_values_conflict (values : Slice Str) : Result Bool := do\n  sorry\n",
    r'''@[rust_loop_body]
def query_values_conflict_loop.body
  (values : Slice Str) (first : Str) (i : Std.Usize) :
  Result (ControlFlow Std.Usize Bool)
  := do
  let n := Slice.len values
  if i < n
  then
    let v ← Slice.index_usize values i
    let eq ← Str.Insts.CoreCmpPartialEqStr.eq first v
    match eq with
    | true =>
      let i1 ← i + 1#usize
      ok (cont i1)
    | false => ok (done true)
  else ok (done false)

@[rust_loop]
def query_values_conflict_loop
  (values : Slice Str) (first : Str) (i : Std.Usize) : Result Bool := do
  loop (fun i1 => query_values_conflict_loop.body values first i1) i

def query_values_conflict (values : Slice Str) : Result Bool := do
  let n := Slice.len values
  if n < 2#usize
  then ok false
  else
    let first ← Slice.index_usize values 0#usize
    query_values_conflict_loop values first 1#usize
''',
    "query_values_conflict",
)

if "sorry" in src:
    sys.exit("FormKernel.lean still contains sorry")
open(p, "w", encoding="utf-8").write(src)
print(f"      patched form ×{n}")
PYEOF
if grep -q 'sorry' "$OUT/lean/FormKernel.lean"; then
  echo "FAIL  FormKernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/pedradb-http/src/form_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.form"
echo "ok    extract form → $OUT"
