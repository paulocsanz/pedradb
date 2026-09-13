#!/usr/bin/env bash
# Extract production scale_kernel.rs (RFC-0176 P2.1). Does not measure µs.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/scale-kernel"
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
SRC="$ROOT/crates/pedradb-core/src/scale_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas --dest-file "$OUT/scale_kernel.llbc"
)
# The shipped kernel keeps three fns opaque to Aeneas ("no bottoms":
# write_forecast_cut, WriteGrowth::token, WriteStaticCut::token); Aeneas
# still writes a usable partial extract and exits nonzero. Accept the
# partial file; no shipped theorem touches those fns.
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/scale_kernel.llbc"
AE_STATUS=$?
set -e
if [[ $AE_STATUS -ne 0 ]]; then
  if [[ ! -f "$OUT/lean/ScaleKernel.lean" ]]; then
    echo "FAIL  aeneas exit $AE_STATUS without ScaleKernel.lean" >&2
    exit "$AE_STATUS"
  fi
  echo "warn  aeneas exit $AE_STATUS (partial extract: write_forecast_cut, WriteGrowth::token, WriteStaticCut::token left opaque)"
fi
{
  echo "path=crates/pedradb-core/src/scale_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.scale"
echo "ok    extract scale → $OUT"
