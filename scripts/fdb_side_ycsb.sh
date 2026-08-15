#!/usr/bin/env bash
# FDB side of the parity bench: runs the SAME ycsb_a..ycsb_f shapes against a real
# FoundationDB cluster and writes fdb_shaped_peer.json (names join montanha-fdb-compare).
#
# Order of preference:
#   1) FDB's official `benchmark` tool on PATH (best fidelity; parse stdout)
#   2) python `fdb` binding (same op mix; client-bound numbers — label them)
#   3) unavailable stub (CI/lab without FDB stays green; parity gate skips)
#
# Env (same names as the Montanha side so both runs share params):
#   FDB_CLUSTER_FILE            cluster file (required for 1/2)
#   MONTANHA_YCSB_RECORDS       keyspace size   (default 1024)
#   MONTANHA_YCSB_OPS           ops per workload (default 200)
#   MONTANHA_YCSB_PAYLOAD       value bytes      (default 100)
#   MONTANHA_YCSB_DIST          uniform|zipfian  (default uniform)
#   FDB_SIDE_API_VERSION        default 720
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/findings/fdb-side-ycsb-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"
STUB="$OUT/fdb_shaped_peer.json"

write_stub() {
  cat >"$STUB" <<EOF
{
  "bench": "fdb-side-peer",
  "status": "unavailable",
  "note": "$1",
  "benches": [
    {"name": "ycsb_a", "keys_per_s": null},
    {"name": "ycsb_b", "keys_per_s": null},
    {"name": "ycsb_c", "keys_per_s": null},
    {"name": "ycsb_d", "keys_per_s": null},
    {"name": "ycsb_e", "keys_per_s": null},
    {"name": "ycsb_f", "keys_per_s": null}
  ]
}
EOF
  echo "fdb_side_ycsb: stub → $STUB ($1)"
  exit 0
}

[[ -n "${FDB_CLUSTER_FILE:-}" ]] || write_stub "FDB_CLUSTER_FILE not set"

if command -v benchmark >/dev/null 2>&1; then
  # Official FDB benchmark tool. Run each workload; stdout is parsed best-effort.
  N="${MONTANHA_YCSB_OPS:-200}"
  R="${MONTANHA_YCSB_RECORDS:-1024}"
  V="${MONTANHA_YCSB_PAYLOAD:-100}"
  ARGS=(-C "$FDB_CLUSTER_FILE" --txns "$N" --keys "$R" --valsize "$V" --loggroup fdb-parity)
  python3 - "$OUT" benchmark "$N" "${ARGS[@]}" <<'PY'
import json, re, subprocess, sys
from pathlib import Path
out, binpath, n, args = Path(sys.argv[1]), sys.argv[2], int(sys.argv[3]), sys.argv[4:]
benches = []
for wl in ("ycsb_a", "ycsb_b", "ycsb_c", "ycsb_d", "ycsb_e", "ycsb_f"):
    p = subprocess.run([binpath, *args, wl], capture_output=True, text=True)
    txt = p.stdout + p.stderr
    m = re.search(r"([\d.]+)\s+transactions/s", txt) or re.search(r"([\d.]+)\s+ops/s", txt)
    kps = float(m.group(1)) if m else None
    benches.append({"name": wl, "keys_per_s": kps, "tool": "official benchmark"})
report = {
    "bench": "fdb-side-peer",
    "status": "ok",
    "topology": "official FDB `benchmark` tool (label process layout)",
    "durability": "benchmark default commit path",
    "benches": benches,
}
(out / "fdb_shaped_peer.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
print("wrote", out / "fdb_shaped_peer.json")
PY
  exit 0
fi

if ! python3 -c "import fdb" >/dev/null 2>&1; then
  write_stub "neither \`benchmark\` tool nor python fdb binding available"
fi

python3 - "$OUT" <<'PY'
import json, os, random, sys, time
from pathlib import Path

import fdb

out = Path(sys.argv[1])
records = int(os.environ.get("MONTANHA_YCSB_RECORDS", "1024"))
ops = int(os.environ.get("MONTANHA_YCSB_OPS", "200"))
payload = int(os.environ.get("MONTANHA_YCSB_PAYLOAD", "100"))
dist = os.environ.get("MONTANHA_YCSB_DIST", "uniform").lower()
api = int(os.environ.get("FDB_SIDE_API_VERSION", "720"))

fdb.api_version(api)
db = fdb.open(os.environ["FDB_CLUSTER_FILE"])
rng = random.Random(0x5EED0001)

def key(i: int) -> bytes:
    return b"ycsb/%06d" % i

# Seed keyspace in one transaction.
tr = db.create_transaction()
val = b"y" * payload
for i in range(records):
    tr[key(i)] = val
tr.commit().wait()

zipf_cdf = None
if dist == "zipfian":
    theta = 0.99
    acc, cdf = 0.0, []
    for i in range(records):
        acc += 1.0 / ((i + 1) ** theta)
        cdf.append(acc)
    total = cdf[-1]
    zipf_cdf = [c / total for c in cdf]

import bisect

def pick(latest: int) -> int:
    if zipf_cdf is None:
        return rng.randrange(records)
    window = min(latest, records)
    u = rng.random()
    target = u * zipf_cdf[window - 1]
    idx = bisect.bisect_left(zipf_cdf[:window], target)
    return (records - window) + min(idx, window - 1)

def run(name, read_pct, insert_pct, rmw=False, scans=False):
    latest = records
    t0 = time.monotonic()
    done = 0
    for _ in range(ops):
        roll = rng.randrange(100)
        if roll < read_pct:
            tr = db.create_transaction()
            tr[key(pick(latest))].value  # read
        elif roll < read_pct + insert_pct:
            tr = db.create_transaction()
            tr[key(latest)] = val
            tr.commit().wait()
            latest += 1
        elif scans:
            tr = db.create_transaction()
            i = pick(latest)
            _ = tr.get_range(key(i), key(i + 25), limit=25)
        elif rmw:
            tr = db.create_transaction()
            k = key(pick(latest))
            _ = tr[k].value
            nv = bytearray(val)
            nv[-1] = (nv[-1] + 1) % 256
            tr[k] = bytes(nv)
            tr.commit().wait()
        else:
            tr = db.create_transaction()
            tr[key(pick(latest))] = val
            tr.commit().wait()
        done += 1
    wall = max(time.monotonic() - t0, 1e-9)
    return {"name": name, "keys_per_s": round(done / wall, 3), "wall_s": round(wall, 4)}

benches = [
    run("ycsb_a", 50, 0),
    run("ycsb_b", 95, 0),
    run("ycsb_c", 100, 0),
    run("ycsb_d", 95, 5),
    run("ycsb_e", 0, 5, scans=True),
    run("ycsb_f", 50, 0, rmw=True),
]
report = {
    "bench": "fdb-side-peer",
    "status": "ok",
    "topology": "python fdb binding (client-bound; label layout)",
    "durability": "sync commit per mutation",
    "records": records,
    "ops": ops,
    "payload_bytes": payload,
    "dist": dist,
    "benches": benches,
    "honesty": "Python client overhead included; official `benchmark` tool preferred when present.",
}
(out / "fdb_shaped_peer.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
print("wrote", out / "fdb_shaped_peer.json")
PY
