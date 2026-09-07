#!/usr/bin/env bash
# Extract production store membership_kernel.rs (clone of raft).
# elect_claim_banner &'static str bottoms patched (toStr), like world.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/store-membership-kernel"
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
SRC="$ROOT/crates/pedradb-store/src/membership_kernel.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --dest-file "$OUT/store_membership_kernel.llbc" )
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/store_membership_kernel.llbc"
set -e
python3 - "$OUT/lean/StoreMembershipKernel.lean" <<'PYEOF'
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
    """def elect_claim_banner
  (es1 : Bool) (es2 : Bool) (es3 : Bool) : Result Str := do
  sorry
""",
    r'''def elect_claim_banner
  (es1 : Bool) (es2 : Bool) (es3 : Bool) : Result Str := do
  let b ← liveness_admitted es1 es2 es3
  if b
  then ok (toStr "eventual-live es1=1 es2=1 es3=1")
  else ok (toStr "bounded-elect not-eventual")
''',
    "elect_claim_banner",
)
repl(
    """def elect_claim_banner_as_is
  (_es1 : Bool) (_es2 : Bool) (_es3 : Bool) : Result Str := do
  sorry
""",
    r'''def elect_claim_banner_as_is
  (_es1 : Bool) (_es2 : Bool) (_es3 : Bool) : Result Str := do
  ok (toStr "live")
''',
    "elect_claim_banner_as_is",
)
old = "core.cmp.Ord.max.default core.cmp.OrdU64"
new = "core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt"
k = src.count(old)
if k:
    src = src.replace(old, new)
    n += k
    print(f"      patched Ord.max.default ×{k}")
if "sorry" in src:
    sys.exit("StoreMembershipKernel.lean still contains sorry")
open(p, "w", encoding="utf-8").write(src)
print(f"      patched store_membership ×{n}")
PYEOF
if grep -q 'sorry' "$OUT/lean/StoreMembershipKernel.lean"; then
  echo "FAIL  StoreMembershipKernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/pedradb-store/src/membership_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.store_membership"
echo "ok    extract store_membership → $OUT"
