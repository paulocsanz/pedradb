#!/usr/bin/env bash
# Machine-check WAL recover choices (F4 / F14 / CRC).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# RFC-0171 P0.3 / RFC-0174: prove the file rustc links (pair wal_recover).
SRC="$ROOT/crates/pedradb-core/src/wal/recover_kernel.rs"
if [[ -x "${VERUS:-}" ]]; then
  :
elif [[ -x "$HOME/.local/verus/verus-arm64-macos/verus" ]]; then
  VERUS="$HOME/.local/verus/verus-arm64-macos/verus"
elif command -v verus >/dev/null 2>&1; then
  VERUS="$(command -v verus)"
else
  echo "error: verus not found" >&2
  exit 127
fi
echo "verus: $VERUS"
"$VERUS" --version
echo "proving: $SRC"
exec "$VERUS" "$SRC" --crate-type=lib --multiple-errors 10 --time "$@"
