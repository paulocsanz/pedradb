#!/usr/bin/env bash
# Kani proof harness for production wal_ticket_kernel.rs (RFC-0258).
# Proves monotonicity, non-overlapping partition and absence of overflow on tickets.
#
#   ./scripts/kani_wal_ticket.sh
#   ./scripts/kani_wal_ticket.sh --required
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REQUIRED=0
if [[ "${1:-}" == "--required" ]]; then
  REQUIRED=1
  shift
fi
if ! command -v cargo-kani >/dev/null 2>&1; then
  echo "error: cargo-kani not found (cargo install --locked kani-verifier && cargo kani setup)" >&2
  if [[ "$REQUIRED" == "1" ]]; then
    exit 127
  fi
  echo "KANI_RESIDUAL: skip (install kani to run wal_ticket_kernel proofs)"
  exit 0
fi
cargo kani --version
cd "$ROOT"
exec cargo kani -p pedradb-core \
  --harness kani_reserve_frame_monotonic_and_partition \
  --harness kani_reserve_frame_two_steps_continuous \
  --harness kani_pwrite_off_lock_matches_truth_table \
  "$@"
