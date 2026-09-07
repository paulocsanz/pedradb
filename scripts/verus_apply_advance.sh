#!/usr/bin/env bash
# Machine-check the apply-loop kernel (RFC-0053 Y2.2 / F10-apply).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# RFC-0171 P0.3 / RFC-0174: prove the file rustc links, not a twin-cópia.
SRC="$ROOT/crates/pedradb-raft/src/apply_kernel.rs"

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
