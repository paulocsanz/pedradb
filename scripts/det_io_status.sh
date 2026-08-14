#!/usr/bin/env bash
# Universe E — det_io Linux path: CONTRACT-OK on Linux, residual on Darwin.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-}"
DET="${PEDRA_DETERMINISMO_DST:-$ROOT/../determinismo/pedradb-dst}"
report() {
  echo "$1"
  [[ -n "$OUT" ]] && echo "$1" >> "$OUT"
}

: > "${OUT:-/dev/null}"
report "det_io_status $(date -u +%Y-%m-%dT%H:%M:%SZ)"
report "os=$(uname -s)-$(uname -m)"

if [[ ! -d "$DET" ]]; then
  report "DET=missing path=$DET"
  report "C1.1=residual_no_determinismo_tree"
  report "det_io_status OK residual"
  exit 0
fi

if [[ -x "$DET/scripts/blocked_residuals_status.sh" ]]; then
  bash "$DET/scripts/blocked_residuals_status.sh" "${OUT}.blocked" 2>&1 | tee -a "${OUT:-/dev/null}" || true
  report "blocked_residuals_invoked=1"
fi

if [[ -x "$DET/scripts/linux_det_io_ci.sh" ]]; then
  set +e
  bash "$DET/scripts/linux_det_io_ci.sh" 2>&1 | tee "${OUT}.linux_ci" 
  EC=$?
  set -e
  if [[ "$EC" -eq 0 ]]; then
    if grep -qE 'CONTRACT-OK|linux_det_io_ci OK|det_io_proof OK' "${OUT}.linux_ci" 2>/dev/null; then
      report "C1.1=CONTRACT_OR_OK exit=0"
    else
      report "C1.1=ok_with_notes exit=0 (see linux_ci log)"
    fi
  else
    report "C1.1=FAIL exit=$EC"
    exit 1
  fi
else
  report "C1.1=residual_missing_linux_det_io_ci"
fi

report "det_io_status OK"
