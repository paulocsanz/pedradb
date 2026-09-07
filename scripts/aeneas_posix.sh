#!/usr/bin/env bash
# Extract production pedradb-posix/src/lib.rs catalog entries (fdatasync_rc_ok).
# Charon --start-from: rest of lib.rs is syscall/unsafe (Dynamic trait, &raw const).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/posix-kernel"
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
SRC="$ROOT/crates/pedradb-posix/src/lib.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::fdatasync_rc_ok' \
    --start-from 'crate::fdatasync_rc_ok_as_is' \
    --dest-file "$OUT/posix_kernel.llbc"
)
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/posix_kernel.llbc"
{
  echo "path=crates/pedradb-posix/src/lib.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.posix"
echo "ok    extract posix → $OUT"
