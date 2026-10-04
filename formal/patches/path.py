# Post-extract patch for kernel 'path' — moved verbatim from
# scripts/aeneas_path.sh. Applied by `cargo xtask proof path` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/PathKernel.lean']
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

def subn(pat, new, label, count=1):
    global src, n
    src2, k = re.subn(pat, new, src, count=count)
    if k != count and count != 0:
        sys.exit(f"patch target not found: {label} (got {k})")
    src = src2
    n += k

subn(
    r"axiom core\.str\.Str\.find\n"
    r"  \{P : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  Str → P → Result \(Option Std\.Usize\)\n",
    "axiom core.str.Str.find {P : Type} :\n"
    "  Str → P → Result (Option Std.Usize)\n",
    "find axiom",
)
subn(
    r"axiom core\.str\.Str\.split_once\n"
    r"  \{P : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  Str → P → Result \(Option \(Str × Str\)\)\n",
    "axiom core.str.Str.split_once {P : Type} :\n"
    "  Str → P → Result (Option (Str × Str))\n"
    "\n"
    "axiom core.str.Str.rsplit_once {P : Type} :\n"
    "  Str → P → Result (Option (Str × Str))\n",
    "split_once axiom",
)
subn(
    r"axiom core\.str\.Str\.strip_prefix\n"
    r"  \{P : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  Str → P → Result \(Option Str\)\n",
    "axiom core.str.Str.strip_prefix {P : Type} :\n"
    "  Str → P → Result (Option Str)\n",
    "strip_prefix axiom",
)
subn(
    r"core\.str\.Str\.find \(sorry /- Could not find: trait_impl_id: \d+-/ 2#usize\)\s*",
    "core.str.Str.find ",
    "find calls",
    count=0,
)
if "Str.find (sorry" in src:
    sys.exit("patch target not found: find calls")
subn(
    r"core\.str\.Str\.strip_prefix sorry /- Could not find: trait_impl_id: \d+-/[ \n]*",
    "core.str.Str.strip_prefix ",
    "strip_prefix calls",
    count=0,
)
if "strip_prefix sorry" in src:
    sys.exit("patch target not found: strip_prefix calls")

repl(
    "def path_after_authority (rest : Str) : Result Str := do\n  sorry\n",
    r'''def path_after_authority (rest : Str) : Result Str := do
  let o ← core.str.Str.find rest '/'
  match o with
  | none => ok (toStr "/")
  | some i =>
    Str.Insts.CoreOpsIndexIndex.index
      core.ops.range.RangeFromUsize.Insts.CoreSliceIndexSliceIndexStrStr rest
      { start := i }
''',
    "path_after_authority",
)
repl(
    "def strip_http_authority (target : Str) : Result (Option Str) := do\n  sorry\n",
    r'''def strip_http_authority (target : Str) : Result (Option Str) := do
  let o ← strip_http_authority_rest target
  match o with
  | none => ok none
  | some rest =>
    let p ← path_after_authority rest
    ok (some p)
''',
    "strip_http_authority",
)
repl(
    """  (c : strip_uri_fragment.closure) (tupled_args : (Str × Str)) :
  Result Str
  := do
  sorry
""",
    """  (c : strip_uri_fragment.closure) (tupled_args : (Str × Str)) :
  Result Str
  := do
  ok tupled_args.1
""",
    "strip_uri_fragment.closure",
)
repl(
    "def strip_uri_fragment (target : Str) : Result Str := do\n  sorry\n",
    r'''def strip_uri_fragment (target : Str) : Result Str := do
  let o ← core.str.Str.split_once target '#'
  match o with
  | none => ok target
  | some (a, _) => ok a
''',
    "strip_uri_fragment",
)
repl(
    """  (c : split_host_port.closure) (tupled_args : (Str × Str)) : Result Str := do
  sorry
""",
    """  (c : split_host_port.closure) (tupled_args : (Str × Str)) : Result Str := do
  ok tupled_args.2
""",
    "split_host_port.closure",
)
repl(
    "def split_host_port (raw1 : Str) : Result (Str × (Option Str)) := do\n  sorry\n",
    r'''@[rust_loop_body]
def ascii_digits_loop.body (bs : Slice Std.U8) (i : Std.Usize) :
  Result (ControlFlow Std.Usize Bool)
  := do
  let n := Slice.len bs
  if i < n
  then
    let b ← Slice.index_usize bs i
    let d ← core.num.U8.is_ascii_digit b
    match d with
    | true =>
      let i1 ← i + 1#usize
      ok (cont i1)
    | false => ok (done false)
  else ok (done true)

@[rust_loop]
def ascii_digits_loop (bs : Slice Std.U8) (i : Std.Usize) : Result Bool := do
  loop (fun i1 => ascii_digits_loop.body bs i1) i

def split_host_port_colon (s : Str) : Result (Str × (Option Str)) := do
  let o ← core.str.Str.rsplit_once s ':'
  match o with
  | none => ok (s, none)
  | some (h, p) =>
    let e ← core.str.Str.is_empty p
    if e
    then ok (s, none)
    else
      let bs ← core.str.Str.as_bytes p
      let all ← ascii_digits_loop bs 0#usize
      if all then ok (h, some p) else ok (s, none)

def split_host_port (raw1 : Str) : Result (Str × (Option Str)) := do
  let o ← core.str.Str.rsplit_once raw1 '@'
  let s ←
    match o with
    | none => ok raw1
    | some (_, h) => ok h
  let obr ← core.str.Str.strip_prefix s (toStr "[")
  match obr with
  | some rest =>
    let oend ← core.str.Str.find rest ']'
    match oend with
    | some end1 =>
      let end2 ← end1 + 1#usize
      let host ←
        Str.Insts.CoreOpsIndexIndex.index
          core.ops.range.RangeToInclusiveUsize.Insts.CoreSliceIndexSliceIndexStrStr
          s { «end» := end2 }
      let after ←
        Str.Insts.CoreOpsIndexIndex.index
          core.ops.range.RangeFromUsize.Insts.CoreSliceIndexSliceIndexStrStr rest
          { start := end2 }
      let oport ← core.str.Str.strip_prefix after (toStr ":")
      let port ←
        match oport with
        | none => ok none
        | some p =>
          let e ← core.str.Str.is_empty p
          if e then ok none else ok (some p)
      ok (host, port)
    | none => split_host_port_colon s
  | none => split_host_port_colon s
''',
    "split_host_port",
)
repl(
    """  (c : origin_form_path.closure) (tupled_args : (Str × Str)) :
  Result Str
  := do
  sorry
""",
    """  (c : origin_form_path.closure) (tupled_args : (Str × Str)) :
  Result Str
  := do
  ok tupled_args.1
""",
    "origin_form_path.closure",
)
repl(
    "def origin_form_path (target : Str) : Result Str := do\n  sorry\n",
    r'''def origin_form_path (target : Str) : Result Str := do
  let target1 ← strip_uri_fragment target
  let o ← strip_http_authority target1
  let p ←
    match o with
    | some p => ok p
    | none =>
      let o1 ← core.str.Str.strip_prefix target1 (toStr "//")
      match o1 with
      | some rest => path_after_authority rest
      | none => ok target1
  let o2 ← core.str.Str.split_once p '?'
  match o2 with
  | none => ok p
  | some (a, _) => ok a
''',
    "origin_form_path",
)
repl(
    """  (c : origin_form_path_as_is.closure) (tupled_args : (Str × Str)) :
  Result Str
  := do
  sorry
""",
    """  (c : origin_form_path_as_is.closure) (tupled_args : (Str × Str)) :
  Result Str
  := do
  ok tupled_args.1
""",
    "origin_form_path_as_is.closure",
)
repl(
    "def origin_form_path_as_is (target : Str) : Result Str := do\n  sorry\n",
    r'''def origin_form_path_as_is (target : Str) : Result Str := do
  let o ← core.str.Str.split_once target '?'
  match o with
  | none => ok target
  | some (a, _) => ok a
''',
    "origin_form_path_as_is",
)

if "sorry" in src:
    sys.exit("PathKernel.lean still contains sorry")
open(p, "w", encoding="utf-8").write(src)
print(f"      patched path ×{n}")
