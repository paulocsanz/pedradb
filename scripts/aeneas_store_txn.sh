#!/usr/bin/env bash
# Extract production store txn_kernel.rs — the rustc bodies (cartoon Verus
# stand-in deleted; payment is Aeneas of the linked bodies, RFC-0171 P0.3).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/store-txn-kernel"
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
SRC="$ROOT/crates/pedradb-store/src/txn_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas --dest-file "$OUT/store_txn_kernel.llbc"
)
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/store_txn_kernel.llbc"
# Codegen quirk (same class as aeneas_merge.sh do-match): `Ord::max` emits
# the instance where the `lt` function is expected. Green kernels use
# `OrdU64.partialOrdInst.lt`; patch to match (idempotent).
python3 - "$OUT/lean/StoreTxnKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = "core.cmp.Ord.max.default core.cmp.OrdU64 "
new = "core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt "
if old in src:
    n = src.count(old)
    src = src.replace(old, new)
    open(p, "w", encoding="utf-8").write(src)
    print(f"      patched Ord.max lt projection ({n})")
elif new in src:
    print("      Ord.max already patched")
else:
    sys.exit("Ord.max patch target not found")
PYEOF
{
  echo "path=crates/pedradb-store/src/txn_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.store_txn"
echo "ok    extract store_txn → $OUT"
