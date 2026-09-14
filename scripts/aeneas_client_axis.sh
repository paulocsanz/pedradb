#!/usr/bin/env bash
# Extract production client_axis_kernel.rs (RFC-0222 P0.7).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/client-axis-kernel"
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
SRC="$ROOT/crates/pedradb-core/src/client_axis_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas --dest-file "$OUT/client_axis_kernel.llbc"
)
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/client_axis_kernel.llbc"
python3 - "$OUT/lean/ClientAxisKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = "core.cmp.Ord.max.default core.cmp.OrdUsize"
new = "core.cmp.Ord.max.default core.cmp.OrdUsize.partialOrdInst.lt"
n = src.count(old)
if n:
    open(p, "w", encoding="utf-8").write(src.replace(old, new))
    print(f"      patched Ord.max.default ×{n} (lt, not Ord inst)")
PYEOF
{
  echo "path=crates/pedradb-core/src/client_axis_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.client_axis"
echo "ok    extract client_axis → $OUT"
