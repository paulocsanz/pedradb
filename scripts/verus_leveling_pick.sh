#!/usr/bin/env bash
# Machine-check the leveled-compaction selection atom (L0→L1 slice and
# pushdown) — findings/2026-08-31-leveling-kernel-unenrolled, pair
# `leveling_pick`. Split from verus_leveling.sh: the close-tier ladder and
# this atom live in separate files (recursive spec fns in one crate perturb
# the other's nonlinear-arithmetic queries; the split is the fix).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/verus/leveling_pick.rs"

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
