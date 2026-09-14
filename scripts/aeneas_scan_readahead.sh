#!/usr/bin/env bash
# Extract production scan_readahead_kernel.rs (RFC-0222 P0.7).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/scan-readahead-kernel"
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
SRC="$ROOT/crates/pedradb-core/src/scan_readahead_kernel.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas --dest-file "$OUT/scan_readahead_kernel.llbc" )
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/scan_readahead_kernel.llbc"
python3 - "$OUT/lean/ScanReadaheadKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
n = 0
for old, new in (
    ("core.cmp.Ord.max.default core.cmp.OrdU64",
     "core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt"),
    ("core.cmp.Ord.max.default core.cmp.OrdUsize",
     "core.cmp.Ord.max.default core.cmp.OrdUsize.partialOrdInst.lt"),
):
    k = src.count(old)
    if k:
        src = src.replace(old, new)
        n += k
if n:
    open(p, "w", encoding="utf-8").write(src)
    print(f"      patched Ord.max.default ×{n}")
PYEOF
{
  echo "path=crates/pedradb-core/src/scan_readahead_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.scan_readahead"
echo "ok    extract scan_readahead → $OUT"
