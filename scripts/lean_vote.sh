#!/usr/bin/env bash
# Check the Lean theorem over the Aeneas extract of vote_kernel.rs.
# Requires elan toolchain leanprover/lean4:v4.31.0 (see formal/aeneas/PINS.md).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LEAN_DIR="$ROOT/formal/aeneas/lean"
REQUIRED=0
if [[ "${1:-}" == "--required" ]]; then
  REQUIRED=1
fi

export PATH="${HOME}/.elan/bin:${PATH}"
LAKE="${LAKE:-}"
if [[ -z "$LAKE" ]]; then
  LAKE="$(command -v lake || true)"
fi

if [[ -z "$LAKE" ]]; then
  msg="lake not on PATH (install elan + leanprover/lean4:v4.31.0). See formal/aeneas/PINS.md"
  if [[ "$REQUIRED" -eq 1 ]]; then
    echo "FAIL  $msg" >&2
    exit 1
  fi
  echo "skip  $msg"
  exit 0
fi

if [[ ! -f "$LEAN_DIR/Vote.lean" || ! -e "$LEAN_DIR/VoteKernel.lean" ]]; then
  echo "FAIL  formal/aeneas/lean/{Vote,VoteKernel}.lean missing" >&2
  exit 1
fi

echo "      lake=$LAKE"
(cd "$LEAN_DIR" && "$LAKE" build Vote Isolated)
echo "ok    lean Vote + Isolated"
