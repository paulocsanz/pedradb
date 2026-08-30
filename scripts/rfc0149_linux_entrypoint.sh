#!/bin/bash
# RFC-0149 P2.1 — Linux 4 vCPU coluna A, maioria das 17 oficiais median >3×.
# Pedra async (PEDRA_PARITY_ASYNC=1) vs Rocks WriteOptions.sync=false.
echo "=== P149_START $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
uname -a
grep -m1 'model name' /proc/cpuinfo || true
nproc
# p04a ships a prebuilt bench. That binary is older than the crates COPY
# (auto-flush O(CFs) + write_cf_owned group-by-CF). Never use it for P2.1.
rm -f /usr/local/bin/rocks-parity-bench /usr/local/bin/rocks-parity-compare
SRC=/src/pedradb
cd "$SRC" || { echo "RESULT=BOOTSTRAP_FAIL"; sleep infinity; }
export CARGO_TARGET_DIR=/tmp/p149-target
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
OUT=/tmp/p149
mkdir -p "$OUT"

load1() { awk '{print $1}' /proc/loadavg; }
wait_quiet() {
  local tag="$1" c=0
  while [ "$c" -lt 2 ]; do
    l=$(load1)
    if awk -v l="$l" 'BEGIN{exit !(l < 2)}'; then c=$((c + 1)); else c=0; fi
    echo "gate $tag load1=${l} (${c}/2)"
    [ "$c" -lt 2 ] && sleep 20
  done
  echo "QUIET $tag load1=$(load1) $(date -u +%H:%M:%S)"
}
wait_quiet pre

export ROCKS_PARITY_SYNC=0
export ROCKS_YCSB_OPS=2000
export ROCKS_YCSB_DIST=zipfian
export ROCKS_PARITY_BIG=0
export ROCKS_PARITY_RATIO_FLOOR=none
unset PEDRA_PARITY_G1 || true

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

echo "P149_COLUMN A async vs rocks-default(sync=false) majority median>3.0"
echo "split suites: ycsb / deps / kvrocks (engines.rs: extra CFs only when the suite needs them)"

for r in 1 2 3; do
  wait_quiet "r$r"
  echo "=== round $r ycsb-compat $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_compat "$OUT/r$r/async" ycsb || echo "ROUND-FAIL r$r ycsb"
  echo "=== round $r ycsb-rocks $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_rocks "$OUT/r$r/rocks" ycsb || echo "ROUND-FAIL r$r ycsb-rocks"
  echo "=== round $r deps-compat $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_compat "$OUT/r$r/deps" deps || echo "ROUND-FAIL r$r deps"
  echo "=== round $r deps-rocks $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_rocks "$OUT/r$r/rocks-deps" deps || echo "ROUND-FAIL r$r deps-rocks"
  echo "=== round $r kvr-compat $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_compat "$OUT/r$r/kvr" kvrocks || echo "ROUND-FAIL r$r kvr"
  echo "=== round $r rocks-kvr $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_rocks "$OUT/r$r/rocks-kvr" kvrocks || echo "ROUND-FAIL r$r rocks-kvr"
  compare "$OUT/r$r/async/rocks_parity_bench.json" \
    "$OUT/r$r/rocks/rocks_parity_bench.json" "$OUT/r$r/compare"
  compare "$OUT/r$r/deps/rocks_parity_bench.json" \
    "$OUT/r$r/rocks-deps/rocks_parity_bench.json" "$OUT/r$r/compare-deps"
  compare "$OUT/r$r/kvr/rocks_parity_bench.json" \
    "$OUT/r$r/rocks-kvr/rocks_parity_bench.json" "$OUT/r$r/compare-kvr"
done

echo "--- P149 gate majority median>3.0 ---"
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
DEPS = {
    "deps_cache_overwrite",
    "deps_lock_prewrite",
    "deps_mvcc_latest",
    "deps_apply_batch",
    "deps_raftlog",
    "deps_scan",
}

def qps(path):
    try:
        d = json.loads(Path(path).read_text())
    except (OSError, json.JSONDecodeError):
        return {}
    return {b["name"]: float(b["qps"]) for b in d.get("benches", []) if b.get("qps")}

per = {}
for r in (1, 2, 3):
    ycsb_q = qps(out / f"r{r}/async/rocks_parity_bench.json")
    yrocks_q = qps(out / f"r{r}/rocks/rocks_parity_bench.json")
    deps_q = qps(out / f"r{r}/deps/rocks_parity_bench.json")
    drocks_q = qps(out / f"r{r}/rocks-deps/rocks_parity_bench.json")
    kvr_q = qps(out / f"r{r}/kvr/rocks_parity_bench.json")
    rkvr_q = qps(out / f"r{r}/rocks-kvr/rocks_parity_bench.json")
    ratios = {}
    for s in GATED:
        if s in KVR:
            c, p = kvr_q.get(s), rkvr_q.get(s)
        elif s in DEPS:
            c, p = deps_q.get(s), drocks_q.get(s)
        else:
            c, p = ycsb_q.get(s), yrocks_q.get(s)
        if c and p and p > 0:
            ratios[s] = c / p
    per[r] = ratios

print("P149_COLUNA_A rounds=3 peer=rocks-default(sync=false) async=PEDRA_PARITY_ASYNC=1")
over_med = 0
over_min = 0
mins = []
n = 0
for s in GATED:
    vals = [per[r][s] for r in (1, 2, 3) if s in per[r]]
    n += 1
    if len(vals) < 3:
        print(f"  MISS {s:26s} rounds={vals!r}")
        continue
    mn, med = min(vals), statistics.median(vals)
    mins.append(mn)
    if med > 3.0:
        over_med += 1
    if mn > 3.0:
        over_min += 1
    tag = ">3" if med > 3.0 else "  "
    print(
        f"  {tag} {s:26s} min={mn:.3f} median={med:.3f} rounds={[round(v,3) for v in vals]}"
    )

need = (n + 1) // 2  # majority of official 17
ok = over_med >= need and n == 17
print(f"majority median>3.0 = {over_med}/{n} (need >={need}); min>3.0 = {over_min}/{n}")
if ok:
    print(f"RESULT=P149_PASS over_med={over_med}/{n} min_ratio={min(mins):.3f}")
else:
    print(f"RESULT=P149_FAIL over_med={over_med}/{n} min_ratio={min(mins) if mins else 'nan'}")
    sys.exit(1)
PY
rc=$?
echo "RESULT=P149_DONE rc=$rc $(date -u +%Y-%m-%dT%H:%M:%SZ)"
# Keep the container alive so logs can be scraped.
sleep infinity
