#!/usr/bin/env bash
# Extract production store vote_kernel.rs.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/store-vote-kernel"
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
SRC="$ROOT/crates/pedradb-store/src/vote_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas --dest-file "$OUT/store_vote_kernel.llbc"
)
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/store_vote_kernel.llbc"
{
  echo "path=crates/pedradb-store/src/vote_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.store_vote"
# RFC-0053 P40: Aeneas emits Option::eq as an axiom; model it as a match def.
python3 - "$OUT/lean/StoreVoteKernel.lean" <<'PYEOF'
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
    open(sys.argv[1], "w", encoding="utf-8").write(src)
    print("      patched Option::eq as match def (RFC-0053 P40)")
PYEOF
echo "ok    extract store_vote → $OUT"
