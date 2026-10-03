#!/usr/bin/env bash
# Reproduces 1M sorted-ingest smoke benchmark for PedraDB
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BACKEND="${1:-pedradb}"
DIR="${2:-/tmp/scale-pedra}"
ENTRIES="${SCALE_ENTRIES:-1000000}"

mkdir -p "$DIR"
cd "$ROOT/crates/snapshot-bench"
SLIPSTREAM_BENCH_BACKENDS="$BACKEND" \
SLIPSTREAM_BENCH_ENTRIES="$ENTRIES" \
TMPDIR="$DIR" \
cargo bench --bench snapshot_backends --features "$BACKEND" -- 'get_hit|prefix_scan|lookup_100'
