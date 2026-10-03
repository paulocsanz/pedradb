#!/usr/bin/env bash
# Extract production c1_modelo_kernel.rs via the shim crate.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/c1-modelo-kernel"
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
SRC="$ROOT/crates/pedradb-raft/src/c1_modelo_kernel.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --exclude 'crate::membership_kernel::elect_claim_banner' \
    --exclude 'crate::membership_kernel::elect_claim_banner_as_is' \
    --dest-file "$OUT/c1_modelo_kernel.llbc" )
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/c1_modelo_kernel.llbc"
python3 - "$OUT/lean/C1ModeloKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = "core.cmp.Ord.max.default core.cmp.OrdU64"
new = "core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt"
n = src.count(old)
if n:
    open(p, "w", encoding="utf-8").write(src.replace(old, new))
    print(f"      patched Ord.max.default ×{n}")
PYEOF
{
  echo "path=crates/pedradb-raft/src/c1_modelo_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.c1_modelo"
echo "ok    extract c1_modelo → $OUT"
