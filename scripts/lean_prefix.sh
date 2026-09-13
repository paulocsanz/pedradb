#!/usr/bin/env bash
# Lean theorems over the Aeneas extract of prefix.rs (RFC-0170 P0.3).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LEAN_DIR="$ROOT/formal/aeneas/lean"
REQUIRED=0
if [[ "${1:-}" == "--required" ]]; then
  REQUIRED=1
fi

export PATH="${HOME}/.elan/bin:${PATH}"
LAKE="${LAKE:-$(command -v lake || true)}"
if [[ -z "$LAKE" ]]; then
  msg="lake not on PATH. See formal/aeneas/PINS.md"
  if [[ "$REQUIRED" -eq 1 ]]; then
    echo "FAIL  $msg" >&2
    exit 1
  fi
  echo "skip  $msg"
  exit 0
fi

if [[ ! -f "$LEAN_DIR/Prefix.lean" || ! -e "$LEAN_DIR/PrefixKernel.lean" ]]; then
  echo "FAIL  formal/aeneas/lean/{Prefix,PrefixKernel}.lean missing" >&2
  exit 1
fi
if ! grep -q "theorem prefix_exclusive_end_matches_spec" "$LEAN_DIR/Prefix.lean"; then
  echo "FAIL  Prefix.lean missing theorem prefix_exclusive_end_matches_spec" >&2
  exit 1
fi
if grep -q "sorry" "$LEAN_DIR/Prefix.lean"; then
  echo "FAIL  Prefix.lean contains sorry" >&2
  exit 1
fi

echo "      lake=$LAKE"
(cd "$LEAN_DIR" && "$LAKE" build Prefix)
echo "ok    lean Prefix"
