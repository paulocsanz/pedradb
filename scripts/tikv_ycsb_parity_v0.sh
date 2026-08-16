#!/usr/bin/env bash
# TiKV-published YCSB mixes (go-ycsb / official TiKV bench docs) on the
# rocks-parity pair: compat (pedradb-core) vs real RocksDB.
#
# This is NOT a 3-node TiKV cluster run (PD + raftstore + coprocessor).
# It is the engine-level same-schedule pair using the workload mixes TiKV
# documents: A 50/50, B 95/5, C 100r, D read-latest+insert, E short scans,
# F RMW — plus the deps suite (raftstore apply / MVCC / raftdb shapes).
#
# Yahoo workloada defaults: 1 KB records (10×100 B), zipfian. go-ycsb's
# checked-in files say uniform; TiKV docs + Yahoo wiki use zipfian — we
# follow Yahoo/TiKV-docs (requestdistribution=zipfian).
#
# Usage: scripts/tikv_ycsb_parity_v0.sh [out_dir]
# Env: ROCKS_YCSB_RECORDS (4096) ROCKS_YCSB_OPS (2000) ROCKS_YCSB_PAYLOAD (1000)
#      ROCKS_PARITY_FULL_SYNC (0 = TiKV-like fdatasync peer; 1 = same-class)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${1:-$ROOT/findings/tikv-ycsb-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"
export ROCKS_YCSB_RECORDS="${ROCKS_YCSB_RECORDS:-4096}"
export ROCKS_YCSB_OPS="${ROCKS_YCSB_OPS:-2000}"
export ROCKS_YCSB_PAYLOAD="${ROCKS_YCSB_PAYLOAD:-1000}"
export ROCKS_YCSB_DIST="${ROCKS_YCSB_DIST:-zipfian}"
export ROCKS_PARITY_SUITE="${ROCKS_PARITY_SUITE:-ycsb,deps}"
export ROCKS_PARITY_SYNC="${ROCKS_PARITY_SYNC:-1}"
FULL="${ROCKS_PARITY_FULL_SYNC:-0}"
export ROCKS_PARITY_FULL_SYNC="$FULL"
# RFC-0035 P2.2: MVCC+scan ≤2× vs same-class F_FULLFSYNC (do not gate ycsb C/E).
if [ "$FULL" = "1" ]; then
  export ROCKS_PARITY_RATIO_FLOOR="${ROCKS_PARITY_RATIO_FLOOR:-0.5}"
  export ROCKS_PARITY_GATE_SHAPES="${ROCKS_PARITY_GATE_SHAPES:-deps_mvcc_latest,deps_scan}"
fi

echo "tikv_ycsb_parity_v0 → $OUT records=$ROCKS_YCSB_RECORDS ops=$ROCKS_YCSB_OPS payload=$ROCKS_YCSB_PAYLOAD dist=$ROCKS_YCSB_DIST full_sync=$FULL"
cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- "$OUT/compat" compat
cargo run -q --release -p rocksdb-parity-bench --features real --bin rocks-parity-bench -- "$OUT/rocks" rocksdb
export ROCKS_PARITY_PEER="$OUT/rocks/rocks_parity_bench.json"
cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
  "$OUT/compat/rocks_parity_bench.json" "$OUT/compare"
echo "tikv_ycsb_parity_v0 OK → $OUT/compare/compare_report.json"
