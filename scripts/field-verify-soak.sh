#!/usr/bin/env bash
# RFC-0062 P2.6 — `pedra verify` soak. With PEDRA_SOAK_DIR, scrub a real
# host directory (fail-closed). Without it, build a temp DB, verify, and
# print residual_no_host (exit 0) unless SOAK_REQUIRED=1.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
if [[ -n "${PEDRA_SOAK_DIR:-}" ]]; then
  cargo run -q -p pedradb-cli -- verify "$PEDRA_SOAK_DIR"
  echo "C2.soak=ok dir=$PEDRA_SOAK_DIR"
  exit 0
fi
if [[ "${SOAK_REQUIRED:-0}" == "1" ]]; then
  echo "C2.soak=FAIL_no_host"
  exit 1
fi
TMP="$(mktemp -d /tmp/pedra-soak-XXXXXX)"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT
cargo run -q -p pedradb-cli -- demo "$TMP/db" >/dev/null
if cargo run -q -p pedradb-cli -- verify "$TMP/db"; then
  echo "C2.soak=residual_no_host lab_verify=ok"
  exit 0
fi
echo "C2.soak=FAIL_lab_verify"
exit 1
