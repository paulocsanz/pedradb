# Post-extract patch for kernel 'form' — moved verbatim from
# scripts/aeneas_form.sh. Applied by `cargo xtask proof form` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/FormKernel.lean']
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
