#!/usr/bin/env bash
# Machine-check the DST Queued-RPC pin kernel (RFC-0067 P2.1).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-store/verus/rpc_mode.rs"

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
