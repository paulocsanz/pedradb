# Post-extract patch for kernel 'auth' — moved verbatim from
# scripts/aeneas_auth.sh. Applied by `cargo xtask proof auth` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/AuthKernel.lean']
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

src2, k = re.subn(
    r"axiom core\.str\.Str\.split_once\n"
    r"  \{P : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  Str → P → Result \(Option \(Str × Str\)\)\n",
    "axiom core.str.Str.split_once {P : Type} :\n"
    "  Str → P → Result (Option (Str × Str))\n",
    src,
    count=1,
)
if k != 1:
    sys.exit("patch target not found: split_once axiom")
src = src2
n += 1

src2, k = re.subn(
    r"axiom core\.str\.Str\.strip_prefix\n"
    r"  \{P : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  Str → P → Result \(Option Str\)\n",
    "axiom core.str.Str.strip_prefix {P : Type} :\n"
    "  Str → P → Result (Option Str)\n",
    src,
    count=1,
)
if k != 1:
    sys.exit("patch target not found: strip_prefix axiom")
src = src2
n += 1

src2, k = re.subn(
    r"core\.str\.Str\.strip_prefix sorry /- Could not find: trait_impl_id: \d+-/ ",
    "core.str.Str.strip_prefix ",
    src,
)
if k < 1:
    sys.exit("patch target not found: strip_prefix call")
src = src2
n += k

repl(
    "axiom bearer_token_from_value : Str → Result (Option Str)\n",
    r'''axiom core.str.Str.split_once_ws : Str → Result (Option (Str × Str))

def bearer_token_from_value (value : Str) : Result (Option Str) := do
  let v ← core.str.Str.trim value
  let b ← core.str.Str.is_empty v
  if b
  then ok none
  else
    let o ← core.str.Str.split_once_ws v
    match o with
    | some (scheme, rest) =>
      let br ← is_bearer_scheme scheme
      if br
      then
        let tok ← core.str.Str.trim rest
        let e ← core.str.Str.is_empty tok
        if e then ok none else ok (some tok)
      else ok none
    | none =>
      let br ← is_bearer_scheme v
      if br
      then ok none
      else
        let nb ← is_non_bearer_auth_scheme v
        if nb then ok none else ok (some v)
''',
    "bearer_token_from_value",
)

repl(
    """def authorization_matches
  {K : Type} {V : Type} (coreconvertAsRefKStrInst : core.convert.AsRef K Str)
  (coreconvertAsRefVStrInst : core.convert.AsRef V Str)
  (headers : Slice (K × V)) (expected : Str) :
  Result Bool
  := do
  sorry
""",
    r'''@[rust_loop_body]
def authorization_matches_loop.body
  {K : Type} {V : Type} (coreconvertAsRefKStrInst : core.convert.AsRef K Str)
  (coreconvertAsRefVStrInst : core.convert.AsRef V Str)
  (headers : Slice (K × V)) (expected : Str)
  (saw_bearer : Bool) (x_pedra : Option Str) (i : Std.Usize) :
  Result (ControlFlow (Bool × (Option Str) × Std.Usize) Bool)
  := do
  let n := Slice.len headers
  if i < n
  then
    let kv ← Slice.index_usize headers i
    let (k, v) := kv
    let k1 ← coreconvertAsRefKStrInst.as_ref k
    let v1 ← coreconvertAsRefVStrInst.as_ref v
    let is_auth ←
      core.str.Str.eq_ignore_ascii_case k1 (toStr "authorization")
    match is_auth with
    | true =>
      let tok ← bearer_token_from_value v1
      match tok with
      | some t =>
        let eq ← Str.Insts.CoreCmpPartialEqStr.eq t expected
        match eq with
        | true => ok (done true)
        | false =>
          let i1 ← i + 1#usize
          ok (cont (true, x_pedra, i1))
      | none =>
        let i1 ← i + 1#usize
        ok (cont (saw_bearer, x_pedra, i1))
    | false =>
      let is_xp ←
        core.str.Str.eq_ignore_ascii_case k1 (toStr "x-pedra-token")
      let x2 ←
        match is_xp with
        | true =>
          match x_pedra with
          | none => ok (some v1)
          | some _ => ok x_pedra
        | false => ok x_pedra
      let i1 ← i + 1#usize
      ok (cont (saw_bearer, x2, i1))
  else
    match saw_bearer with
    | true => ok (done false)
    | false =>
      let b ←
        core.option.Option.Insts.CoreCmpPartialEqOption.eq
          Str.Insts.CoreCmpPartialEqStr x_pedra (some expected)
      ok (done b)

@[rust_loop]
def authorization_matches_loop
  {K : Type} {V : Type} (coreconvertAsRefKStrInst : core.convert.AsRef K Str)
  (coreconvertAsRefVStrInst : core.convert.AsRef V Str)
  (headers : Slice (K × V)) (expected : Str)
  (saw_bearer : Bool) (x_pedra : Option Str) (i : Std.Usize) :
  Result Bool
  := do
  loop
    (fun (saw1, x1, i1) =>
      authorization_matches_loop.body coreconvertAsRefKStrInst
        coreconvertAsRefVStrInst headers expected saw1 x1 i1)
    (saw_bearer, x_pedra, i)

def authorization_matches
  {K : Type} {V : Type} (coreconvertAsRefKStrInst : core.convert.AsRef K Str)
  (coreconvertAsRefVStrInst : core.convert.AsRef V Str)
  (headers : Slice (K × V)) (expected : Str) :
  Result Bool
  := do
  authorization_matches_loop coreconvertAsRefKStrInst
    coreconvertAsRefVStrInst headers expected false none 0#usize
''',
    "authorization_matches",
)

repl(
    """def authorization_matches_as_is
  (headers : Slice (Str × Str)) (expected : Str) : Result Bool := do
  sorry
""",
    r'''@[rust_loop_body]
def authorization_matches_as_is_loop.body
  (headers : Slice (Str × Str)) (expected : Str) (i : Std.Usize) :
  Result (ControlFlow Std.Usize Bool)
  := do
  let n := Slice.len headers
  if i < n
  then
    let kv ← Slice.index_usize headers i
    let (k, v) := kv
    let is_auth ← core.str.Str.eq_ignore_ascii_case k (toStr "authorization")
    match is_auth with
    | true =>
      let tok ← bearer_token_from_value_as_is v
      match tok with
      | some t =>
        let eq ← Str.Insts.CoreCmpPartialEqStr.eq t expected
        ok (done eq)
      | none =>
        let is_xp ←
          core.str.Str.eq_ignore_ascii_case k (toStr "x-pedra-token")
        match is_xp with
        | true =>
          let eq ← Str.Insts.CoreCmpPartialEqStr.eq v expected
          ok (done eq)
        | false =>
          let i1 ← i + 1#usize
          ok (cont i1)
    | false =>
      let is_xp ←
        core.str.Str.eq_ignore_ascii_case k (toStr "x-pedra-token")
      match is_xp with
      | true =>
        let eq ← Str.Insts.CoreCmpPartialEqStr.eq v expected
        ok (done eq)
      | false =>
        let i1 ← i + 1#usize
        ok (cont i1)
  else ok (done false)

@[rust_loop]
def authorization_matches_as_is_loop
  (headers : Slice (Str × Str)) (expected : Str) (i : Std.Usize) :
  Result Bool
  := do
  loop (fun i1 => authorization_matches_as_is_loop.body headers expected i1) i

def authorization_matches_as_is
  (headers : Slice (Str × Str)) (expected : Str) : Result Bool := do
  authorization_matches_as_is_loop headers expected 0#usize
''',
    "authorization_matches_as_is",
)

if "sorry" in src:
    sys.exit("AuthKernel.lean still contains sorry")
open(p, "w", encoding="utf-8").write(src)
print(f"      patched auth ×{n}")
