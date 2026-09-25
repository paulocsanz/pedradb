#!/usr/bin/env bash
# RFC-0278: Miri Concurrency, Aliasing (Tree-Borrows), and Data-Race Gate
#
# Validates that concurrent atomic primitives and lockless synchronization
# execute with zero undefined behavior (UB), zero aliasing violations under
# Tree Borrows, and zero data races.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "=========================================================="
echo "   RFC-0278: Miri Concurrency & Data-Race Soundness Gate  "
echo "=========================================================="

export MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"

echo "Running Miri on atomic synchronization and concurrency primitives..."
cargo +nightly miri test -q -p pedradb-core --test lsm_bisimulation_invariance
cargo +nightly miri test -q -p pedradb-core --test rfc0279_confluence_liveness_ssi
cargo +nightly miri test -q -p pedradb-core --test rfc0280_vlog_ram_iter_hlc_space
cargo +nightly miri test -q -p pedradb-core --test rfc0281_bloom_dual_log_tombstone_comparator_dio
cargo +nightly miri test -q -p pedradb-core --test rfc0282_dez_pilares_verificacao
cargo +nightly miri test -q -p pedradb-core --test rfc0283_dez_pilares_segunda_onda
cargo +nightly miri test -q -p pedradb-core --test rfc0284_dez_pilares_terceira_onda
cargo +nightly miri test -q -p pedradb-core --test rfc0285_cinco_pilares_motor_puro
cargo +nightly miri test -q -p pedradb-core --test rfc0285_dez_pilares_caixote_metal_federation

echo "=========================================================="
echo "   GATE miri_concurrency: GREEN (0 Data-Races, 0 UB)      "
echo "=========================================================="
