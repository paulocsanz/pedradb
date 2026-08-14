#!/usr/bin/env bash
# RFC-0020 P0.1 — in-tree silent_wrong gate (fixed seed matrix).
# Optional: if ../determinismo/pedradb-dst is present, also run sibling gate.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "== pedradb in-tree silent_wrong_gate =="
cargo run -q -p pedradb-dst --bin silent_wrong_gate

echo "== denser dst unit gate =="
cargo test -q -p pedradb-dst rfc20_silent_wrong_gate_matrix -- --nocapture
cargo test -q -p pedradb-dst denser_sweep_silent_wrong_zero -- --nocapture

DET="${PEDRA_DETERMINISMO_DST:-$ROOT/../determinismo/pedradb-dst}"
if [[ -x "$DET/scripts/ci_silent_wrong_gate.sh" ]]; then
  echo "== sibling determinismo ci_silent_wrong_gate (optional) =="
  # Best-effort: do not fail pedradb gate if sibling harness is incomplete.
  set +e
  bash "$DET/scripts/ci_silent_wrong_gate.sh"
  DET_EC=$?
  set -e
  if [[ "$DET_EC" -ne 0 ]]; then
    echo "WARN: determinismo gate exited $DET_EC (in-tree gate still authoritative)" >&2
  fi
else
  echo "note: sibling determinismo gate not found at $DET (in-tree only)"
fi

echo "ci_silent_wrong_gate OK (in-tree silent_wrong=0)"
