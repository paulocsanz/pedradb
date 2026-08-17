#!/usr/bin/env bash
# RFC-0040 P1.1: three-column remesura — Pedra · Rocks sync · Rocks async.
# Multi-client (default 4) so group commit can amortize fdatasync.
#
# Usage: scripts/tikv_ycsb_parity_mc_async.sh [out_dir]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${1:-$ROOT/findings/rfc0040-p11}"
mkdir -p "$OUT"
export ROCKS_YCSB_RECORDS="${ROCKS_YCSB_RECORDS:-4096}"
export ROCKS_YCSB_OPS="${ROCKS_YCSB_OPS:-2000}"
export ROCKS_YCSB_PAYLOAD="${ROCKS_YCSB_PAYLOAD:-1000}"
export ROCKS_YCSB_DIST="${ROCKS_YCSB_DIST:-zipfian}"
export ROCKS_PARITY_SUITE="${ROCKS_PARITY_SUITE:-deps}"
export ROCKS_PARITY_CLIENTS="${ROCKS_PARITY_CLIENTS:-4}"
export ROCKS_PARITY_FULL_SYNC=0

echo "tikv_ycsb_parity_mc_async → $OUT records=$ROCKS_YCSB_RECORDS ops=$ROCKS_YCSB_OPS clients=$ROCKS_PARITY_CLIENTS suite=$ROCKS_PARITY_SUITE"

cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- "$OUT/compat" compat

export ROCKS_PARITY_SYNC=1
cargo run -q --release -p rocksdb-parity-bench --features real --bin rocks-parity-bench -- "$OUT/rocks-sync" rocksdb

export ROCKS_PARITY_SYNC=0
cargo run -q --release -p rocksdb-parity-bench --features real --bin rocks-parity-bench -- "$OUT/rocks-async" rocksdb

python3 - "$OUT" <<'PY'
import json, pathlib, sys
out = pathlib.Path(sys.argv[1])

def load(p):
    return json.loads((out / p / "rocks_parity_bench.json").read_text())

def qps(doc):
    return {b["name"]: b for b in doc["benches"] if "qps" in b}

c, s, a = load("compat"), load("rocks-sync"), load("rocks-async")
cq, sq, aq = qps(c), qps(s), qps(a)
names = [n for n in cq if n in sq and n in aq]
print(f"OFFICIAL peer = rocks default (sync=false). Pedra still fdatasyncs.")
print(f"{'shape':28s}  pedra     p50    rocks_default  p50    vs_default  rocks_sync  p50    vs_sync")
for n in names:
    p, rs, ra = cq[n], sq[n], aq[n]
    vs_def = p["qps"] / ra["qps"] if ra["qps"] else float("nan")
    vs_s = p["qps"] / rs["qps"] if rs["qps"] else float("nan")
    print(f"{n:28s}  {p['qps']:8.0f}  {p['p50_ms']:6.3f}  {ra['qps']:13.0f}  {ra['p50_ms']:6.3f}  {vs_def:6.2f}x  {rs['qps']:10.0f}  {rs['p50_ms']:6.3f}  {vs_s:6.2f}x")
PY
echo "tikv_ycsb_parity_mc_async OK → $OUT"
