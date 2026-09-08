#!/usr/bin/env bash
# Machine-check the durable-term step (RFC-0158 P0.2 / F125/F127).
# Twin of crates/pedradb-raft/src/vote_kernel.rs — do not link into production.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# RFC-0171 P0.3 / RFC-0174: prove the file rustc links (pair durable_term).
SRC="$ROOT/crates/pedradb-raft/src/vote_kernel.rs"

# Prefer explicit install location used on this machine; else PATH.
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
# Standalone twin is a library crate (no main).
exec "$VERUS" "$SRC" --crate-type=lib --multiple-errors 10 --time "$@"
