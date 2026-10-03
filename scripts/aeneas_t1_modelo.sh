#!/usr/bin/env bash
# Extract production t1_modelo_kernel.rs via the shim crate.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/t1-modelo-kernel"
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
SRC="$ROOT/crates/pedradb-store/src/t1_modelo_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas --dest-file "$OUT/t1_modelo_kernel.llbc"
)
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/t1_modelo_kernel.llbc"
python3 - "$OUT/lean/T1ModeloKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = "core.cmp.Ord.max.default core.cmp.OrdU64"
new = "core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt"
n = src.count(old)
if n:
    open(p, "w", encoding="utf-8").write(src.replace(old, new))
    print(f"      patched Ord.max.default ×{n} (lt, not Ord inst)")
PYEOF
{
  echo "path=crates/pedradb-store/src/t1_modelo_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.t1_modelo"
echo "ok    extract t1_modelo → $OUT"
