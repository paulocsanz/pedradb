#!/usr/bin/env bash
# Machine-check the pure RequestVote decision (RFC-0002 P1.4 / F15).
# Twin of crates/pedradb-raft/src/vote_kernel.rs — do not link into production.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-raft/verus/vote_decision.rs"

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
