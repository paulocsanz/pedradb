#!/usr/bin/env bash
# Adversarial audit parity runs (2026-08-21). Two columns:
#   g1    — compat default (fdatasync before Ok) vs RocksDB default (sync=false). Product claim (RFC-0041 / AGENTS.md).
#   async — PEDRA_PARITY_ASYNC=1 compat vs RocksDB default. RFC-0044 claim under audit (5x most / >=95% all).
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
OUT="findings/2026-08-21-adversarial-audit"
mkdir -p "$OUT"
# Scoped for this shared box (ambient load ~50): default suite (ycsb,deps =
# the 16 official shapes), 30k ops. Volume ≈ 0.5 GB per engine phase.
export ROCKS_YCSB_RECORDS=1024
export ROCKS_YCSB_OPS=30000
export ROCKS_YCSB_PAYLOAD=1000
export ROCKS_YCSB_DIST=zipfian
export ROCKS_PARITY_CLIENTS=4
export ROCKS_PARITY_SYNC=0

echo "=== load before ==="; uptime

echo "=== column g1: compat (G1) ==="
cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- "$OUT/g1/compat" compat 2>&1 | tail -3
rm -rf "$OUT/g1/compat/db-compat"
echo "=== column g1: rocksdb default ==="
cargo run -q --release -p rocksdb-parity-bench --features real --bin rocks-parity-bench -- "$OUT/g1/rocks" rocksdb 2>&1 | tail -3
rm -rf "$OUT/g1/rocks/db-rocksdb"
ROCKS_PARITY_PEER="$OUT/g1/rocks/rocks_parity_bench.json" \
  cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
  "$OUT/g1/compat/rocks_parity_bench.json" "$OUT/g1/compare"
echo "g1 compare exit: $?"

echo "=== column async: compat (PEDRA_PARITY_ASYNC=1) ==="
PEDRA_PARITY_ASYNC=1 \
  cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- "$OUT/async/compat" compat 2>&1 | tail -3
rm -rf "$OUT/async/compat/db-compat"
echo "=== column async: rocksdb default ==="
cargo run -q --release -p rocksdb-parity-bench --features real --bin rocks-parity-bench -- "$OUT/async/rocks" rocksdb 2>&1 | tail -3
rm -rf "$OUT/async/rocks/db-rocksdb"
ROCKS_PARITY_PEER="$OUT/async/rocks/rocks_parity_bench.json" \
  cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
  "$OUT/async/compat/rocks_parity_bench.json" "$OUT/async/compare"
echo "async compare exit: $?"

echo "=== load after ==="; uptime
echo "AUDIT-PARITY-DONE"
