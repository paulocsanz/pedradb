#!/usr/bin/env bash
# Lean theorems over the Aeneas extract of write_admission_kernel.rs (RFC-0170 P2.1).
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

if [[ ! -f "$LEAN_DIR/WriteAdmission.lean" || ! -e "$LEAN_DIR/WriteAdmissionKernel.lean" ]]; then
  echo "FAIL  formal/aeneas/lean/{WriteAdmission,WriteAdmissionKernel}.lean missing" >&2
  exit 1
fi
if ! grep -q "theorem write_admission_idle_matches_spec" "$LEAN_DIR/WriteAdmission.lean"; then
  echo "FAIL  WriteAdmission.lean missing theorem write_admission_idle_matches_spec" >&2
  exit 1
fi
if grep -q "sorry" "$LEAN_DIR/WriteAdmission.lean"; then
  echo "FAIL  WriteAdmission.lean contains sorry" >&2
  exit 1
fi

echo "      lake=$LAKE"
(cd "$LEAN_DIR" && "$LAKE" build WriteAdmission)
echo "ok    lean WriteAdmission"
