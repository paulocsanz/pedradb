#!/usr/bin/env bash
# Machine-check iterator window retain (RFC-0151 P1).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# RFC-0171 P0.3 / RFC-0174: prove the file rustc links, not a twin-cópia.
SRC="$ROOT/crates/rocksdb-compat/src/iter_kernel.rs"
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
