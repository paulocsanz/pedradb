#!/usr/bin/env bash
# Machine-check txn_commit_action / discard_cut (RFC-0002 P9 / F47).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-store/verus/txn_kernel.rs"
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
