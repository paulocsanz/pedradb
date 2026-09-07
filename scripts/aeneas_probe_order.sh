#!/usr/bin/env bash
# Extract production probe_order_kernel.rs catalog entries (first_probe_on_equal_lo).
# Charon --start-from: probe_order walk is Iterator-refused (whole-file CFailure).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/probe-order-kernel"
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
SRC="$ROOT/crates/pedradb-core/src/probe_order_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::first_probe_on_equal_lo' \
    --start-from 'crate::first_probe_on_equal_lo_as_is' \
    --dest-file "$OUT/probe_order_kernel.llbc"
)
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/probe_order_kernel.llbc"
{
  echo "path=crates/pedradb-core/src/probe_order_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.probe_order"
echo "ok    extract probe_order → $OUT"
