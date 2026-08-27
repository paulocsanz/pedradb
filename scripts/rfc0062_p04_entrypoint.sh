#!/bin/bash
# RFC-0062 P0.4 — Linux coluna A after LAST_CF write-through + pwrite on main.
# Isolated raftlog first (the P0.3 fail). Then full 3-round suite.
echo "=== P04_START $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
uname -a
grep -m1 'model name' /proc/cpuinfo || true
nproc
SRC=/src/pedradb
cd "$SRC" || { echo "RESULT=BOOTSTRAP_FAIL"; sleep infinity; }
export CARGO_TARGET_DIR=/tmp/p04-target
export CARGO_PROFILE_RELEASE_DEBUG=0
echo "--- build offline (j3) started $(date -u +%H:%M:%S) ---"
cargo build --release --offline -p rocksdb-parity-bench --features real -j 3 \
  || { echo "RESULT=BUILD_FAIL"; sleep infinity; }
cp "$CARGO_TARGET_DIR/release/rocks-parity-bench" /tmp/rocks-parity-bench
cp "$CARGO_TARGET_DIR/release/rocks-parity-compare" /tmp/rocks-parity-compare 2>/dev/null || true
rm -rf "$CARGO_TARGET_DIR"
echo "RESULT=BUILD_OK $(date -u +%H:%M:%S)"
BIN=/tmp/rocks-parity-bench
CMP=/tmp/rocks-parity-compare
OUT=/tmp/p04
mkdir -p "$OUT"

load1() { awk '{print $1}' /proc/loadavg; }
c=0
while [ "$c" -lt 2 ]; do
  l=$(load1)
  if awk -v l="$l" 'BEGIN{exit !(l < 2)}'; then c=$((c + 1)); else c=0; fi
  echo "gate load1=${l} (${c}/2)"
  [ "$c" -lt 2 ] && sleep 20
done
echo "QUIET load1=$(load1) $(date -u +%H:%M:%S)"

export ROCKS_PARITY_SYNC=0
export ROCKS_YCSB_OPS=2000
export ROCKS_YCSB_DIST=zipfian
export ROCKS_PARITY_BIG=0

run_compat() {
  local dest="$1" suite="${2:-}" only="${3:-}"
  [ -n "$suite" ] && export ROCKS_PARITY_SUITE="$suite"
  [ -n "$only" ] && export ROCKS_PARITY_ONLY="$only"
  PEDRA_PARITY_ASYNC=1 "$BIN" "$dest" compat
  local rc=$?
  unset ROCKS_PARITY_SUITE ROCKS_PARITY_ONLY
  rm -rf "$dest/db-compat" "$dest/db-rocksdb"
  return $rc
}

run_rocks() {
  local dest="$1" suite="${2:-}" only="${3:-}"
  [ -n "$suite" ] && export ROCKS_PARITY_SUITE="$suite"
  [ -n "$only" ] && export ROCKS_PARITY_ONLY="$only"
  ROCKS_PARITY_SYNC=0 "$BIN" "$dest" rocksdb
  local rc=$?
  unset ROCKS_PARITY_SUITE ROCKS_PARITY_ONLY
  rm -rf "$dest/db-compat" "$dest/db-rocksdb"
  return $rc
}

compare() {
  local compat="$1" peer="$2" dest="$3"
  if [ -x "$CMP" ]; then
    ROCKS_PARITY_PEER="$peer" "$CMP" "$compat" "$dest" || echo "COMPARE-FAILED $dest"
  fi
}

# Isolated raftlog first — P0.3 fail was min 0.958 here.
for r in 1 2 3; do
  echo "=== isolated r$r raftlog-compat $(date -u +%H:%M:%S) ==="
  run_compat "$OUT/iso$r/raft-pedra" ycsb,deps deps_raftlog || echo "ISO-FAIL raft pedra r$r"
  echo "=== isolated r$r raftlog-rocks $(date -u +%H:%M:%S) ==="
  run_rocks "$OUT/iso$r/raft-rocks" ycsb,deps deps_raftlog || echo "ISO-FAIL raft rocks r$r"
done
python3 - "$OUT" <<'PY'
import json, statistics, sys
from pathlib import Path
out = Path(sys.argv[1])
def qps(path):
    try:
        d = json.loads(Path(path).read_text())
    except (OSError, json.JSONDecodeError):
        return {}
    return {b["name"]: float(b["qps"]) for b in d.get("benches", []) if b.get("qps")}
def lat(path, name):
    try:
        d = json.loads(Path(path).read_text())
    except (OSError, json.JSONDecodeError):
        return None
    for b in d.get("benches", []):
        if b.get("name") == name:
            return b
    return None
rs = []
print("P04_ISOLATED deps_raftlog empty-DB (LAST_CF write-through)")
for r in (1, 2, 3):
    p = qps(out / f"iso{r}/raft-pedra/rocks_parity_bench.json").get("deps_raftlog")
    q = qps(out / f"iso{r}/raft-rocks/rocks_parity_bench.json").get("deps_raftlog")
    pb = lat(out / f"iso{r}/raft-pedra/rocks_parity_bench.json", "deps_raftlog")
    qb = lat(out / f"iso{r}/raft-rocks/rocks_parity_bench.json", "deps_raftlog")
    if p and q and q > 0:
        rs.append(p / q)
        extra = ""
        if pb and qb:
            extra = (
                f" p50_us={pb.get('p50_ms',0)*1000:.1f}/{qb.get('p50_ms',0)*1000:.1f}"
                f" p99_us={pb.get('p99_ms',0)*1000:.1f}/{qb.get('p99_ms',0)*1000:.1f}"
            )
        print(f"  ISO deps_raftlog r{r} ratio={p/q:.3f}{extra}")
if rs:
    mn, med = min(rs), statistics.median(rs)
    print(f"  ISO deps_raftlog min={mn:.3f} median={med:.3f}")
    print(f"RESULT=P04_ISO_{'PASS' if mn > 1.0 else 'FAIL'} min={mn:.3f}")
else:
    print("RESULT=P04_ISO_FAIL missing")
PY

for r in 1 2 3; do
  echo "=== round $r async-compat $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_compat "$OUT/r$r/async" || echo "ROUND-FAIL r$r async"
  echo "=== round $r rocks-default $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_rocks "$OUT/r$r/rocks" || echo "ROUND-FAIL r$r rocks"
  echo "=== round $r kvr-compat $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_compat "$OUT/r$r/kvr" kvrocks || echo "ROUND-FAIL r$r kvr"
  echo "=== round $r rocks-kvr $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_rocks "$OUT/r$r/rocks-kvr" kvrocks || echo "ROUND-FAIL r$r rocks-kvr"
  compare "$OUT/r$r/async/rocks_parity_bench.json" \
    "$OUT/r$r/rocks/rocks_parity_bench.json" "$OUT/r$r/compare"
  compare "$OUT/r$r/kvr/rocks_parity_bench.json" \
    "$OUT/r$r/rocks-kvr/rocks_parity_bench.json" "$OUT/r$r/compare-kvr"
done

echo "--- P0.4 gate min>1.0 ---"
python3 - "$OUT" <<'PY'
import json, statistics, sys
from pathlib import Path

out = Path(sys.argv[1])
GATED = [
    "ycsb_a", "ycsb_b", "ycsb_c", "ycsb_d", "ycsb_e", "ycsb_f",
    "deps_cache_overwrite", "deps_lock_prewrite", "deps_mvcc_latest",
    "deps_apply_batch", "deps_raftlog", "deps_scan",
    "kvrocks_get", "kvrocks_set", "kvrocks_scan", "kvrocks_pipelined_set",
    "kvrocks_blob_set",
]
KVR = {"kvrocks_get", "kvrocks_set", "kvrocks_scan", "kvrocks_pipelined_set", "kvrocks_blob_set"}

def qps(path):
    try:
        d = json.loads(Path(path).read_text())
    except (OSError, json.JSONDecodeError):
        return {}
    return {b["name"]: float(b["qps"]) for b in d.get("benches", []) if b.get("qps")}

per = {}
for r in (1, 2, 3):
    async_q = qps(out / f"r{r}/async/rocks_parity_bench.json")
    rocks_q = qps(out / f"r{r}/rocks/rocks_parity_bench.json")
    kvr_q = qps(out / f"r{r}/kvr/rocks_parity_bench.json")
    rkvr_q = qps(out / f"r{r}/rocks-kvr/rocks_parity_bench.json")
    ratios = {}
    for s in GATED:
        if s in KVR:
            c, p = kvr_q.get(s), rkvr_q.get(s)
        else:
            c, p = async_q.get(s), rocks_q.get(s)
        if c and p and p > 0:
            ratios[s] = c / p
    per[r] = ratios

print("P04_COLUNA_A rounds=3 peer=rocks-default(sync=false) async=PEDRA_PARITY_ASYNC=1 BIG=0")
fails = []
mins = []
for s in GATED:
    vals = [per[r][s] for r in (1, 2, 3) if s in per[r]]
    if len(vals) < 3:
        print(f"  FAIL {s:26s} missing rounds={vals!r}")
        fails.append(s)
        continue
    mn, med = min(vals), statistics.median(vals)
    mins.append(mn)
    ok = mn > 1.0
    print(f"  {'PASS' if ok else 'FAIL'} {s:26s} min={mn:.3f} median={med:.3f} rounds={[round(v,3) for v in vals]}")
    if not ok:
        fails.append(s)

if fails:
    print(f"RESULT=P04_FAIL min_ratio={min(mins) if mins else 'nan'} fail={','.join(fails)}")
    sys.exit(1)
print(f"RESULT=P04_PASS min_ratio={min(mins):.3f}")
PY
rc=$?
echo "RESULT=P04_DONE rc=$rc $(date -u +%Y-%m-%dT%H:%M:%SZ)"
mkdir -p /data
echo "rc=$rc ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)" >/data/p04-last-result.txt 2>/dev/null || true
sleep infinity
