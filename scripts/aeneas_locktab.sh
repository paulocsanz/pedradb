#!/usr/bin/env bash
# Extract production locktab.rs catalog entries (wait_for_deadlock).
# Charon --start-from + --exclude LockTable: lock is nested-borrows sorry.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/locktab-kernel"
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
SRC="$ROOT/crates/rocksdb-compat/src/locktab_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::wait_for_deadlock' \
    --start-from 'crate::wait_for_deadlock_as_is' \
    --exclude 'crate::LockTable' \
    --exclude 'crate::{LockTable}' \
    --exclude 'crate::LockTable::lock' \
    --exclude 'crate::LockTable::unlock_all' \
    --exclude 'crate::LockTable::new' \
    --exclude 'crate::LockTable::alloc_id' \
    --dest-file "$OUT/locktab_kernel.llbc"
)
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/locktab_kernel.llbc"
{
  echo "path=crates/rocksdb-compat/src/locktab_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.locktab"
echo "ok    extract locktab → $OUT"
