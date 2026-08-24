#!/usr/bin/env bash
# RFC-0053 P2.1: Lean theorems over Aeneas extracts of ae_kernel / commit_kernel.
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
  msg="lake not on PATH (install elan + leanprover/lean4:v4.31.0). See formal/aeneas/PINS.md"
  if [[ "$REQUIRED" -eq 1 ]]; then
    echo "FAIL  $msg" >&2
    exit 1
  fi
  echo "skip  $msg"
  exit 0
fi

for f in Ae.lean AeKernel.lean Commit.lean CommitKernel.lean; do
  if [[ ! -e "$LEAN_DIR/$f" ]]; then
    echo "FAIL  formal/aeneas/lean/$f missing" >&2
    exit 1
  fi
done

echo "      lake=$LAKE"
(cd "$LEAN_DIR" && "$LAKE" build Ae Commit)
echo "ok    lean Ae + Commit"
