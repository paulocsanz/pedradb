#!/usr/bin/env bash
# End-to-end parity run: Montanha ycsb suite + (optional) FDB side + ratio gate.
#
# Usage:
#   scripts/montanha_fdb_parity_v0.sh [out_dir]
# Env:
#   MONTANHA_PARITY_RATIO_FLOOR  gate floor (default 0.5; set "none" to report-only)
#   FDB_CLUSTER_FILE             when set, runs scripts/fdb_side_ycsb.sh against real FDB
#   MONTANHA_YCSB_*              shared by both sides (records/ops/payload/dist)
#   MONTANHA_WRITE_BACKPRESSURE  passes through to the Montanha run
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OUT="${1:-$ROOT/findings/fdb-parity-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"

FLOOR="${MONTANHA_PARITY_RATIO_FLOOR:-0.5}"
if [[ "$FLOOR" == "none" ]]; then
  unset MONTANHA_PARITY_RATIO_FLOOR || true
else
  export MONTANHA_PARITY_RATIO_FLOOR="$FLOOR"
fi

echo "montanha_fdb_parity_v0 → $OUT floor=${FLOOR} fdb_side=${FDB_CLUSTER_FILE:-absent}"
MONTANHA_BENCH_SUITE=ycsb \
  cargo run -q -p pedradb-store --release --bin montanha-fdb-bench -- "$OUT/montanha"

if [[ -n "${FDB_CLUSTER_FILE:-}" ]]; then
  bash scripts/fdb_side_ycsb.sh "$OUT/fdb_side"
  export MONTANHA_FDB_PEER="$OUT/fdb_side/fdb_shaped_peer.json"
else
  echo "FDB_CLUSTER_FILE unset — compare in template mode (no ratios gated)"
  unset MONTANHA_FDB_PEER || true
fi

cargo run -q -p pedradb-store --release --bin montanha-fdb-compare -- \
  "$OUT/montanha/fdb_shaped_bench.json" "$OUT/compare"

python3 - "$OUT/compare/compare_report.json" "$FLOOR" <<'PY'
import json, sys
from pathlib import Path
r = json.loads(Path(sys.argv[1]).read_text())
floor = sys.argv[2]
p = r.get("parity") or {}
rows = [x for x in (r.get("ratios") or []) if x.get("montanha_over_fdb") is not None]
print(
    "parity v0:",
    "floor=", p.get("floor"),
    "shapes_with_peer=", p.get("shapes_with_peer"),
    "min_ratio=", p.get("min_ratio"),
    "pass=", p.get("pass"),
)
for x in rows:
    print("  ", x["shape"], "montanha/fdb=", x["montanha_over_fdb"], "meets_floor=", x.get("meets_floor"))
assert "parity" in r and "ratios" in r, r
if floor != "none" and p.get("pass") is False:
    sys.exit(2)
PY
echo "montanha_fdb_parity_v0 OK → $OUT/compare/compare_report.json"
