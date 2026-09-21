#!/usr/bin/env bash
# Extract production pedradb-posix/src/lib.rs catalog entries (fdatasync_rc_ok)
# plus EINTR retry refusal. Charon --start-from: rest of lib.rs is
# syscall/unsafe. SOURCE sha256 is git HEAD (concurrent clippy on
# filesystem_available_bytes is not in the stamp).
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
SRC="$ROOT/crates/pedradb-posix/src/lib_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::fdatasync_rc_ok' \
    --start-from 'crate::fdatasync_rc_ok_as_is' \
    --start-from 'crate::fdatasync_eintr_retry_admitted' \
    --start-from 'crate::fdatasync_eintr_retry_admitted_as_is' \
    --dest-file "$OUT/posix_kernel.llbc"
)
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/posix_kernel.llbc"
{
  echo "path=crates/pedradb-posix/src/lib_kernel.rs"
  echo "sha256=$(git -C "$ROOT" show HEAD:crates/pedradb-posix/src/lib_kernel.rs | shasum -a 256 | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.posix"
echo "ok    extract posix → $OUT"
