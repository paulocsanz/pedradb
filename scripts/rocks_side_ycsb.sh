#!/usr/bin/env bash
# Real-RocksDB side of the rocks parity pair — same YCSB shapes as the compat side.
#
# Usage:
#   scripts/rocks_side_ycsb.sh [out_dir]
# Env:
#   ROCKS_YCSB_*     shared knobs (records/ops/payload/dist) — same as compat side
#   ROCKS_PARITY_SYNC  1 (default) = sync per write (matches Pedra fsync-before-Ok);
#                    0 = RocksDB async-WAL default (reference run, label differs)
# On build/run failure writes an honest all-null stub (status "unavailable").
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OUT="${1:-$ROOT/findings/rocks-side-ycsb-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"
PEER="$OUT/rocks_shaped_peer.json"

SYNC="${ROCKS_PARITY_SYNC:-1}"
echo "rocks_side_ycsb → $OUT sync=$SYNC records=${ROCKS_YCSB_RECORDS:-1024} ops=${ROCKS_YCSB_OPS:-200}"

if cargo run -q --release -p rocksdb-parity-bench --features real --bin rocks-parity-bench -- \
  "$OUT/run" rocksdb 2>"$OUT/build.err"; then
  cp "$OUT/run/rocks_parity_bench.json" "$PEER"
  echo "rocks_side_ycsb OK → $PEER"
else
  echo "rocks_side_ycsb: real engine failed (see $OUT/build.err) — honest stub" >&2
  cat >"$PEER" <<EOF
{
  "bench": "rocks-side-peer",
  "engine": "rocksdb",
  "status": "unavailable",
  "sync": null,
  "durability": null,
  "note": "cargo build --features real failed; fill qps manually or fix build",
  "benches": [
    {"name": "ycsb_a", "qps": null},
    {"name": "ycsb_b", "qps": null},
    {"name": "ycsb_c", "qps": null},
    {"name": "ycsb_d", "qps": null},
    {"name": "ycsb_e", "qps": null},
    {"name": "ycsb_f", "qps": null}
  ]
}
EOF
  echo "rocks_side_ycsb stub → $PEER"
fi
