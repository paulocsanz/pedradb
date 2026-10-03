#!/usr/bin/env bash
# Extract production vote_kernel.rs via the include crate.
# Not a prover. Fails if Charon/Aeneas are required and missing.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/vote-kernel"
OUT="$ROOT/formal/aeneas/out"
REQUIRED=0
if [[ "${1:-}" == "--required" ]]; then
  REQUIRED=1
fi

CHARON="${CHARON:-}"
AENEAS="${AENEAS:-}"
if [[ -z "$CHARON" ]]; then
  CHARON="$(command -v charon || true)"
fi
if [[ -z "$AENEAS" ]]; then
  AENEAS="$(command -v aeneas || true)"
fi

if [[ -z "$CHARON" || -z "$AENEAS" ]]; then
  msg="charon/aeneas not on PATH (set CHARON= AENEAS=). See formal/aeneas/PINS.md"
  if [[ "$REQUIRED" -eq 1 ]]; then
    echo "FAIL  $msg" >&2
    exit 1
  fi
  echo "skip  $msg"
  exit 0
fi

mkdir -p "$OUT"
echo "      charon=$CHARON"
echo "      aeneas=$AENEAS"
SRC="$ROOT/crates/pedradb-raft/src/vote_kernel.rs"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas --dest-file "$OUT/vote_kernel.llbc"
)
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/vote_kernel.llbc"
{
  echo "path=crates/pedradb-raft/src/vote_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.vote"

# RFC-0053 P40: Aeneas emits Option::eq as an axiom; model it as a match def
# instead (no new TCB axioms), then keep the lake copy in sync.
python3 - "$OUT/lean/VoteKernel.lean" "$ROOT/formal/aeneas/lean/VoteKernel.lean" <<'PYEOF'
import sys
axiom = (
    "axiom core.option.Option.Insts.CoreCmpPartialEqOption.eq\n"
    "  {T : Type} (cmpPartialEqInst : core.cmp.PartialEq T T) :\n"
    "  Option T → Option T → Result Bool\n"
)
defn = (
    "def core.option.Option.Insts.CoreCmpPartialEqOption.eq\n"
    "  {T : Type} (cmpPartialEqInst : core.cmp.PartialEq T T) :\n"
    "  Option T → Option T → Result Bool\n"
    "  := fun a b =>\n"
    "    match a, b with\n"
    "    | some x, some y => cmpPartialEqInst.eq x y\n"
    "    | none, none => ok true\n"
    "    | _, _ => ok false\n"
)
src = open(sys.argv[1], encoding="utf-8").read()
if axiom in src:
    src = src.replace(axiom, defn, 1)
elif defn not in src:
    sys.exit("Option::eq patch target not found in regenerated extract")
open(sys.argv[1], "w", encoding="utf-8").write(src)
open(sys.argv[2], "w", encoding="utf-8").write(src)
print("      patched Option::eq as match def (RFC-0053 P40)")
PYEOF
echo "ok    extract → $OUT"
