#!/usr/bin/env bash
# Machine-check sst_crc_fate (RFC-0077 P2.1). scan_guard stays F167.
# Not a zero-glue theorem (P2.2).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/verus/sst_crc_fate.rs"

if [[ -x "${VERUS:-}" ]]; then
  :
elif [[ -x "$HOME/.local/verus/verus-arm64-macos/verus" ]]; then
  VERUS="$HOME/.local/verus/verus-arm64-macos/verus"
elif command -v verus >/dev/null 2>&1; then
  VERUS="$(command -v verus)"
else
  echo "error: verus not found (install to ~/.local/verus/verus-arm64-macos or set VERUS=)" >&2
  exit 127
fi

echo "verus: $VERUS"
"$VERUS" --version
echo "proving: $SRC"
exec "$VERUS" "$SRC" --crate-type=lib --multiple-errors 10 --time "$@"
