#!/usr/bin/env bash
# Extract production fail_closed.rs catalog entries (parse_error_writes_status).
# Charon --start-from: rest of the file uses eq_ignore_ascii_case (str/pattern).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/fail-closed-kernel"
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
SRC="$ROOT/crates/pedradb-http/src/fail_closed.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::parse_error_writes_status' \
    --start-from 'crate::parse_error_writes_status_as_is' \
    --dest-file "$OUT/fail_closed_kernel.llbc" )
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/fail_closed_kernel.llbc"
{
  echo "path=crates/pedradb-http/src/fail_closed.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.fail_closed"
echo "ok    extract fail_closed → $OUT"
