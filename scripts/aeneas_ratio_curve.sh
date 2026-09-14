#!/usr/bin/env bash
# Extract production ratio_curve_kernel.rs (RFC-0222 P0.7).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/ratio-curve-kernel"
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
SRC="$ROOT/crates/pedradb-core/src/ratio_curve_kernel.rs"
echo "      charon=$CHARON"
# Whole-file extract hits an Aeneas lifetime-constraint CFailure on
# `&'static str` fields (GetSideAnchor). --start-from isolates the
# integer cold-fraction predicate the rest of the curve is built on.
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::ratio_curve_kernel::cold_permille' \
    --dest-file "$OUT/ratio_curve_kernel.llbc" )
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/ratio_curve_kernel.llbc"
if [[ ! -f "$OUT/lean/RatioCurveKernel.lean" ]]; then
  echo "FAIL  RatioCurveKernel.lean missing" >&2
  exit 1
fi
python3 - "$OUT/lean/RatioCurveKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
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
  echo "path=crates/pedradb-core/src/ratio_curve_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.ratio_curve"
echo "ok    extract ratio_curve → $OUT"
