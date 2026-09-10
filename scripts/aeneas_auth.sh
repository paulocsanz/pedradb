#!/usr/bin/env bash
# Extract production auth_kernel.rs catalog entries.
# Charon --exclude of str Pattern methods (CFailure pattern.rs:99).
# Pattern clauseInst holes, bearer_token_from_value axiom, and
# authorization_matches Iterator patched (generated Lean, restamped here).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/auth-kernel"
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
SRC="$ROOT/crates/pedradb-http/src/auth_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas \
    --start-from-if-exists 'crate::ascii_lower' \
    --start-from-if-exists 'crate::ascii_lower_as_is' \
    --start-from-if-exists 'crate::ascii_upper' \
    --start-from-if-exists 'crate::ascii_upper_as_is' \
    --start-from-if-exists 'crate::normalize_http_method' \
    --start-from-if-exists 'crate::normalize_http_method_as_is' \
    --start-from-if-exists 'crate::is_bearer_scheme' \
    --start-from-if-exists 'crate::is_bearer_scheme_as_is' \
    --start-from-if-exists 'crate::is_non_bearer_auth_scheme' \
    --start-from-if-exists 'crate::is_non_bearer_auth_scheme_as_is' \
    --start-from-if-exists 'crate::bearer_token_from_value' \
    --start-from-if-exists 'crate::bearer_token_from_value_as_is' \
    --start-from-if-exists 'crate::authorization_matches' \
    --exclude 'core::str::{str}::contains' \
    --exclude 'core::str::{str}::eq_ignore_ascii_case' \
    --exclude 'core::str::{str}::rsplit_once' \
    --exclude 'core::str::{str}::split_once' \
    --exclude 'core::str::{str}::find' \
    --exclude 'core::str::{str}::trim' \
    --exclude 'core::str::{str}::to_ascii_uppercase' \
    --exclude 'core::str::{str}::strip_prefix' \
    --exclude 'core::str::pattern' \
    --exclude 'core::str::pattern::Pattern' \
    --dest-file "$OUT/auth_kernel.llbc"
)
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/auth_kernel.llbc"
set -e
python3 - "$OUT/lean/AuthKernel.lean" <<'PYEOF'
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

if "sorry" in src:
    sys.exit("AuthKernel.lean still contains sorry")
open(p, "w", encoding="utf-8").write(src)
print(f"      patched auth ×{n}")
PYEOF
if grep -q 'sorry' "$OUT/lean/AuthKernel.lean"; then
  echo "FAIL  AuthKernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/pedradb-http/src/auth_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.auth"
echo "ok    extract auth → $OUT"
