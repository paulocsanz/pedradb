#!/usr/bin/env bash
# RFC-0052 P2.2 — TCG guest status. Named residual, never fake-green.
#
# Without a guest image (`PEDRA_QEMU_SSH` unset / unreachable) this script
# prints `C2.2=residual_no_guest` and exits 0. That is **not**
# "native trace_hash == TCG trace_hash" (RFC-0052 P2.1) and it is **not**
# a wall-clock comparison (forbidden).
#
# TCG_REQUIRED=1 (only if a gate must have a guest): missing SSH is a
# failure. Default CI leaves this unset so the residual stays honest.
#
# Sibling: `../determinismo/pedradb-dst/scripts/qemu_subset_revalidate.sh`
# (RFC-0005 / RFC-0018). This wrapper is the Pedra-side status line.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${1:-}"
report() {
  echo "$1"
  if [[ -n "$OUT" ]]; then
    echo "$1" >> "$OUT"
  fi
}

if [[ -n "$OUT" ]]; then
  : > "$OUT"
fi

report "tcg_guest_status $(date -u +%Y-%m-%dT%H:%M:%SZ)"
report "os=$(uname -s)-$(uname -m)"
report "native_cmd=cargo run -q -p pedradb-world --bin world_smoke -- 42"
report "p2_1=scripts/tcg_world_smoke.sh (Linux TCG guest; C2.1=native_eq_guest)"
report "compare_wall_clock=forbidden"

if [[ -n "${PEDRA_QEMU_SSH:-}" ]]; then
  report "PEDRA_QEMU_SSH=$PEDRA_QEMU_SSH"
  if ssh $PEDRA_QEMU_SSH 'echo qemu_guest_ok' >/dev/null 2>&1; then
    report "C2.2=guest_reachable"
    report "note=SSH up; native-vs-guest trace_hash is RFC-0052 P2.1 (not this script)"
    report "tcg_guest_status OK reachable"
    exit 0
  fi
  report "C2.2=FAIL_ssh_unreachable"
  exit 1
fi

if [[ "${TCG_REQUIRED:-}" == 1 ]]; then
  report "C2.2=FAIL_no_guest"
  echo "tcg_guest_status: PEDRA_QEMU_SSH required (TCG_REQUIRED=1)" >&2
  exit 1
fi

report "C2.2=residual_no_guest"
report "note=no PEDRA_QEMU_SSH; do not treat this as TCG_PASS"
report "tcg_guest_status OK residual"
exit 0
