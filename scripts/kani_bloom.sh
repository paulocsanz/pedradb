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
# The crate's #[cfg(kani)] harnesses are exactly the four bloom proofs;
# run every proof in pedradb-core.
exec cargo kani -p pedradb-core "$@"
