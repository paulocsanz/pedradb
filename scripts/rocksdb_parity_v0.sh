#!/usr/bin/env bash
# End-to-end rocks parity: compat bench + (optional) real-RocksDB side + ratio gate.
#
# Usage:
#   scripts/rocksdb_parity_v0.sh [out_dir]
# Env:
#   ROCKS_PARITY_RATIO_FLOOR  gate floor (default "none"; 0.5 = RFC-0031 2×)
#   ROCKS_PARITY_GATE_SHAPES  csv of shapes the floor applies to (default all;
#                             writes: ycsb_a,ycsb_b,ycsb_d,ycsb_f,deps_apply_batch,deps_raftlog,deps_cache_overwrite)
#   ROCKS_PARITY_TEMPLATE     "1" skips the real side (CI mode; ratios stay null)
#   ROCKS_PARITY_SYNC         1 = WriteOptions.sync (default), 0 = async WAL
#   ROCKS_PARITY_FULL_SYNC    1 = F_FULLFSYNC on Rocks WAL (same class as Pedra)
#   ROCKS_YCSB_*              shared knobs (records/ops/payload/dist)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OUT="${1:-$ROOT/findings/rocks-parity-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"

FLOOR="${ROCKS_PARITY_RATIO_FLOOR:-none}"
if [[ "$FLOOR" == "none" ]]; then
  unset ROCKS_PARITY_RATIO_FLOOR || true
else
  export ROCKS_PARITY_RATIO_FLOOR="$FLOOR"
fi

echo "rocksdb_parity_v0 → $OUT floor=$FLOOR template=${ROCKS_PARITY_TEMPLATE:-0} sync=${ROCKS_PARITY_SYNC:-1}"
cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- "$OUT/compat" compat

if [[ "${ROCKS_PARITY_TEMPLATE:-0}" == "1" ]]; then
  echo "ROCKS_PARITY_TEMPLATE=1 — compare in template mode (no real side, ratios null)"
  unset ROCKS_PARITY_PEER || true
else
  bash scripts/rocks_side_ycsb.sh "$OUT/rocks_side"
  export ROCKS_PARITY_PEER="$OUT/rocks_side/rocks_shaped_peer.json"
fi

cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
  "$OUT/compat/rocks_parity_bench.json" "$OUT/compare"

python3 - "$OUT/compare/compare_report.json" "$FLOOR" <<'PY'
import json, sys
from pathlib import Path
r = json.loads(Path(sys.argv[1]).read_text())
floor = sys.argv[2]
p = r.get("parity") or {}
rows = [x for x in (r.get("ratios") or []) if x.get("compat_over_rocksdb") is not None]
print(
    "rocks parity v0:",
    "floor=", p.get("floor"),
    "shapes_with_peer=", p.get("shapes_with_peer"),
    "min_ratio=", p.get("min_ratio"),
    "pass=", p.get("pass"),
    "rocksdb.sync=", r.get("rocksdb", {}).get("sync"),
)
for x in rows:
    print("  ", x["shape"], "compat/rocksdb=", x["compat_over_rocksdb"], "meets_floor=", x.get("meets_floor"))
assert "parity" in r and "ratios" in r, r
if floor != "none" and p.get("pass") is False:
    sys.exit(2)
PY
echo "rocksdb_parity_v0 OK → $OUT/compare/compare_report.json"
