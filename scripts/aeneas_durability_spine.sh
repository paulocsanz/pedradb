#!/usr/bin/env bash
# Extract production durability_spine_kernel.rs (RFC-0222 P0.7).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/durability-spine-kernel"
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
SRC="$ROOT/crates/pedradb-core/src/durability_spine_kernel.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas --dest-file "$OUT/durability_spine_kernel.llbc" )
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/durability_spine_kernel.llbc"
python3 - "$OUT/lean/DurabilitySpineKernel.lean" <<'PYEOF'
import sys, os
p = sys.argv[1]
if not os.path.isfile(p):
    # Aeneas may name the crate file after the crate, not the inner module.
    import glob
    cands = glob.glob(os.path.join(os.path.dirname(p), "*Spine*.lean")) + \
            glob.glob(os.path.join(os.path.dirname(p), "*Durability*.lean"))
    print("      generated:", cands)
    raise SystemExit(0)
src = open(p, encoding="utf-8").read()
n = 0
for old, new in (
    ("core.cmp.Ord.max.default core.cmp.OrdU64",
     "core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt"),
    ("core.cmp.Ord.min.default core.cmp.OrdU64",
     "core.cmp.Ord.min.default core.cmp.OrdU64.partialOrdInst.lt"),
):
    k = src.count(old)
    if k:
        src = src.replace(old, new)
        n += k
if n:
    open(p, "w", encoding="utf-8").write(src)
    print(f"      patched Ord.max/min.default ×{n}")
PYEOF
{
  echo "path=crates/pedradb-core/src/durability_spine_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.durability_spine"
echo "ok    extract durability_spine → $OUT"
