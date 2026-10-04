#!/usr/bin/env bash
# Kani harnesses on production recover_kernel.rs (RFC-0166 P0.3).
# Finite enums, straight-line matches — exhaustive, no extra unwind.
#
#   ./scripts/kani_recover.sh
#   ./scripts/kani_recover.sh --required
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
  echo "KANI_RESIDUAL: skip (install kani to run recover_kernel proofs)"
  exit 0
fi
cargo kani --version
cd "$ROOT"
exec cargo kani -p pedradb-core \
  --harness from_record_type_is_total_bijection \
  --harness fragment_act_contract_and_divergence \
  --harness is_length_resyncable_exact_class \
  "$@"
