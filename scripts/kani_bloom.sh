#!/usr/bin/env bash
# Kani proof harnesses for the production Bloom filter (T1–T4).
# First Kani use in pedradb-core: bit-precise check of the compiled code
# (wrapping arithmetic included). See docs/formal/bloom-filter-theorems.md.
#
#   ./scripts/kani_bloom.sh            # run if cargo-kani is installed
#   ./scripts/kani_bloom.sh --required # fail if cargo-kani is missing
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
  echo "KANI_RESIDUAL: skip (install kani to run the bloom T1–T4 proofs)"
  exit 0
fi
cargo kani --version
cd "$ROOT"
# T1 (tiny 64-bit/k=2 filter, 2-byte symbolic key): ~90 s SAT, 2959 checks.
# T4 (inactive never rejects): <1 s, 337 checks.
# T2 encode/decode and T3 symbolic-header OOM or stall on this host —
# they stay in `src/bloom.rs` for a bigger machine. Exhaustive tests +
# Verus cover those ∀-style on their domains.
exec cargo kani -p pedradb-core \
  --harness insert_then_may_contain_all_keys \
  --harness inactive_filter_never_rejects \
  "$@"
