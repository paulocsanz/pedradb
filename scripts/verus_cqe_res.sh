#!/usr/bin/env bash
# Machine-check cqe_res_ok on the file rustc links (single artifact —
# not the twin-cópia). Not a ring model (P2.2).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-io-uring/src/cqe_kernel.rs"

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
