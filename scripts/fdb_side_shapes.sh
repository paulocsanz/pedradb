#!/usr/bin/env bash
# Optional FDB-side probes for montanha-fdb-compare (lab only).
#
# Requires: fdbcli on PATH, FDB_CLUSTER_FILE set, writable cluster.
# Does NOT claim parity — records wall times for a few fdbcli shapes.
#
# Usage:
#   FDB_CLUSTER_FILE=/path/to/fdb.cluster ./scripts/fdb_side_shapes.sh [out_dir]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/findings/fdb-side-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"

if [[ -z "${FDB_CLUSTER_FILE:-}" ]]; then
  echo "FDB_CLUSTER_FILE not set — writing unavailable stub"
  cat >"$OUT/fdb_shaped_peer.json" <<'EOF'
{
  "bench": "fdb-side-peer",
  "status": "unavailable",
  "note": "Set FDB_CLUSTER_FILE and install fdbcli; re-run scripts/fdb_side_shapes.sh"
}
EOF
  exit 0
fi
if ! command -v fdbcli >/dev/null 2>&1; then
  echo "fdbcli not on PATH"
  exit 1
fi

C=(-C "$FDB_CLUSTER_FILE")
N="${FDB_SIDE_N:-20}"
echo "fdb_side_shapes n=$N → $OUT"

# Warm
fdbcli "${C[@]}" --exec "status minimal" >/dev/null || true

# Point set/get loop (sync via fdbcli writemode + set)
# Note: fdbcli is not a high-QPS harness; this is a hook, not YCSB.
T0=$(date +%s.%N)
for i in $(seq 1 "$N"); do
  fdbcli "${C[@]}" --exec "writemode on; set mcmp/k$i v$i" >/dev/null
done
T1=$(date +%s.%N)
PUT_WALL=$(python3 -c "print($T1 - $T0)")
PUT_KPS=$(python3 -c "print($N / max($PUT_WALL, 1e-9))")

T0=$(date +%s.%N)
for i in $(seq 1 "$N"); do
  fdbcli "${C[@]}" --exec "get mcmp/k$i" >/dev/null
done
T1=$(date +%s.%N)
GET_WALL=$(python3 -c "print($T1 - $T0)")
GET_KPS=$(python3 -c "print($N / max($GET_WALL, 1e-9))")

python3 - <<PY
import json, time
from pathlib import Path
out = Path("$OUT")
report = {
    "bench": "fdb-side-peer",
    "status": "ok",
    "topology": "lab fdbcli (label process layout yourself)",
    "durability": "fdbcli default sync commit path",
    "n": int("$N"),
    "benches": [
        {"name": "A1_raw_put", "keys_per_s": float("$PUT_KPS"), "wall_s": float("$PUT_WALL"),
         "note": "fdbcli set loop — not production FDB QPS"},
        {"name": "A2_raw_get", "keys_per_s": float("$GET_KPS"), "wall_s": float("$GET_WALL"),
         "note": "fdbcli get loop"},
    ],
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "honesty": "fdbcli is a slow client; use bindings for serious QPS",
}
(out / "fdb_shaped_peer.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
print("wrote", out / "fdb_shaped_peer.json")
print("Next: MONTANHA_FDB_PEER=$OUT/fdb_shaped_peer.json cargo run -p pedradb-store --release --bin montanha-fdb-compare -- findings/fdb-bench-scale-s10/fdb_shaped_bench.json findings/fdb-compare-local")
PY
