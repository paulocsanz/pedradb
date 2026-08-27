#!/bin/bash
# RFC-0062 P1.1 — Linux coluna B: Pedra G1 vs Rocks WriteOptions.sync=true.
# Same-class fdatasync (FULL_SYNC=0). min(3) > 1.0 all official shapes.
echo "=== P11_START $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
uname -a
grep -m1 'model name' /proc/cpuinfo || true
nproc
# Slim runtime images only have the prebuilt binary (no rustc / source).
if [ -x /usr/local/bin/rocks-parity-bench ]; then
  echo "--- using prebuilt /usr/local/bin/rocks-parity-bench $(date -u +%H:%M:%S) ---"
  cp /usr/local/bin/rocks-parity-bench /tmp/rocks-parity-bench
  cp /usr/local/bin/rocks-parity-compare /tmp/rocks-parity-compare 2>/dev/null || true
  echo "RESULT=BUILD_OK prebuilt $(date -u +%H:%M:%S)"
else
  SRC=/src/pedradb
  cd "$SRC" || { echo "RESULT=BOOTSTRAP_FAIL"; sleep infinity; }
  export CARGO_TARGET_DIR=/tmp/p11-target
  export CARGO_PROFILE_RELEASE_DEBUG=0
  echo "--- build offline (j3) started $(date -u +%H:%M:%S) ---"
  cargo build --release --offline -p rocksdb-parity-bench --features real -j 3 \
    || { echo "RESULT=BUILD_FAIL"; sleep infinity; }
  cp "$CARGO_TARGET_DIR/release/rocks-parity-bench" /tmp/rocks-parity-bench
  cp "$CARGO_TARGET_DIR/release/rocks-parity-compare" /tmp/rocks-parity-compare 2>/dev/null || true
  rm -rf "$CARGO_TARGET_DIR"
  echo "RESULT=BUILD_OK $(date -u +%H:%M:%S)"
fi
BIN=/tmp/rocks-parity-bench
CMP=/tmp/rocks-parity-compare
OUT=/tmp/p11
mkdir -p "$OUT"

load1() { awk '{print $1}' /proc/loadavg; }
# p11h r3 ran at load1=4.56 (ycsb_c r1=21×). Wait between suite rounds.
wait_quiet() {
  local tag="$1" c=0
  while [ "$c" -lt 3 ]; do
    l=$(load1)
    if awk -v l="$l" 'BEGIN{exit !(l < 1.5)}'; then c=$((c + 1)); else c=0; fi
    echo "gate $tag load1=${l} (${c}/3)"
    [ "$c" -lt 3 ] && sleep 15
  done
  echo "QUIET $tag load1=$(load1) $(date -u +%H:%M:%S)"
}
wait_quiet pre

unset PEDRA_PARITY_ASYNC
export PEDRA_PARITY_G1=1
export ROCKS_PARITY_SYNC=1
export ROCKS_PARITY_FULL_SYNC=0
export ROCKS_PARITY_ALLOW_SYNC_PEER=1
export ROCKS_PARITY_RATIO_FLOOR=none
export ROCKS_YCSB_OPS=2000
export ROCKS_YCSB_DIST=zipfian
export ROCKS_PARITY_BIG=0

run_compat() {
  local dest="$1" suite="${2:-}" only="${3:-}"
  [ -n "$suite" ] && export ROCKS_PARITY_SUITE="$suite"
  [ -n "$only" ] && export ROCKS_PARITY_ONLY="$only"
  env -u PEDRA_PARITY_ASYNC PEDRA_PARITY_G1=1 "$BIN" "$dest" compat
  local rc=$?
  unset ROCKS_PARITY_SUITE ROCKS_PARITY_ONLY
  rm -rf "$dest/db-compat" "$dest/db-rocksdb"
  return $rc
}

run_rocks() {
  local dest="$1" suite="${2:-}" only="${3:-}"
  [ -n "$suite" ] && export ROCKS_PARITY_SUITE="$suite"
  [ -n "$only" ] && export ROCKS_PARITY_ONLY="$only"
  ROCKS_PARITY_SYNC=1 ROCKS_PARITY_FULL_SYNC=0 "$BIN" "$dest" rocksdb
  local rc=$?
  unset ROCKS_PARITY_SUITE ROCKS_PARITY_ONLY
  rm -rf "$dest/db-compat" "$dest/db-rocksdb"
  return $rc
}

echo "P11_COLUMN B G1 vs rocks sync=true (fdatasync class, FULL_SYNC=0)"

for r in 1 2 3; do
  wait_quiet "iso$r"
  echo "=== isolated r$r ycsb_a-compat $(date -u +%H:%M:%S) ==="
  run_compat "$OUT/iso$r/a-pedra" ycsb ycsb_a || echo "ISO-FAIL a pedra r$r"
  echo "=== isolated r$r ycsb_a-rocks $(date -u +%H:%M:%S) ==="
  run_rocks "$OUT/iso$r/a-rocks" ycsb ycsb_a || echo "ISO-FAIL a rocks r$r"
  echo "=== isolated r$r raftlog-compat $(date -u +%H:%M:%S) ==="
  run_compat "$OUT/iso$r/raft-pedra" ycsb,deps deps_raftlog || echo "ISO-FAIL raft pedra r$r"
  echo "=== isolated r$r raftlog-rocks $(date -u +%H:%M:%S) ==="
  run_rocks "$OUT/iso$r/raft-rocks" ycsb,deps deps_raftlog || echo "ISO-FAIL raft rocks r$r"
done
python3 - "$OUT" <<'PY'
import json, statistics, sys
from pathlib import Path
out = Path(sys.argv[1])

def bench(path, name):
    try:
        d = json.loads(Path(path).read_text())
    except (OSError, json.JSONDecodeError):
        return None
    for b in d.get("benches", []):
        if b.get("name") == name:
            return d.get("sync"), d.get("durability"), b
    return None

print("P11_ISOLATED ycsb_a / deps_raftlog")
for shape, folder in (("ycsb_a", "a"), ("deps_raftlog", "raft")):
    rs = []
    for r in (1, 2, 3):
        p = bench(out / f"iso{r}/{folder}-pedra/rocks_parity_bench.json", shape)
        q = bench(out / f"iso{r}/{folder}-rocks/rocks_parity_bench.json", shape)
        if not p or not q or not q[2].get("qps"):
            print(f"  ISO {shape} r{r} missing")
            continue
        pb, qb = p[2], q[2]
        ratio = pb["qps"] / qb["qps"]
        rs.append(ratio)
        print(
            f"  ISO {shape} r{r} ratio={ratio:.3f}"
            f" p50_us={pb.get('p50_ms',0)*1000:.1f}/{qb.get('p50_ms',0)*1000:.1f}"
            f" sync={p[0]}/{q[0]}"
        )
        if r == 1:
            print(f"    dur P={p[1][:80] if p[1] else None}")
            print(f"    dur R={q[1][:80] if q[1] else None}")
    if rs:
        mn = min(rs)
        print(f"  ISO {shape:26s} min={mn:.3f} median={statistics.median(rs):.3f}")
        print(f"RESULT=P11_ISO_{shape}_{'PASS' if mn > 1.0 else 'FAIL'} min={mn:.3f}")
PY

for r in 1 2 3; do
  wait_quiet "r$r"
  echo "=== round $r g1-compat $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_compat "$OUT/r$r/g1" || echo "ROUND-FAIL r$r g1"
  echo "=== round $r rocks-sync $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_rocks "$OUT/r$r/rocks" || echo "ROUND-FAIL r$r rocks"
  echo "=== round $r kvr-g1 $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_compat "$OUT/r$r/kvr" kvrocks || echo "ROUND-FAIL r$r kvr"
  echo "=== round $r rocks-kvr $(date -u +%H:%M:%S) load1=$(load1) ==="
  run_rocks "$OUT/r$r/rocks-kvr" kvrocks || echo "ROUND-FAIL r$r rocks-kvr"
done

echo "--- P1.1 gate min>1.0 coluna B ---"
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

def meta(path):
    try:
        d = json.loads(Path(path).read_text())
        return d.get("sync"), d.get("durability")
    except (OSError, json.JSONDecodeError):
        return None, None

def qps(path):
    try:
        d = json.loads(Path(path).read_text())
    except (OSError, json.JSONDecodeError):
        return {}
    return {b["name"]: float(b["qps"]) for b in d.get("benches", []) if b.get("qps")}

ps, pd = meta(out / "r1/g1/rocks_parity_bench.json")
rs, rd = meta(out / "r1/rocks/rocks_parity_bench.json")
print(f"P11_COLUNA_B Pedra sync={ps} Rocks sync={rs}")
print(f"  P dur={pd}")
print(f"  R dur={rd}")
if ps is not True or rs is not True:
    print("RESULT=P11_FAIL wrong_sync_column")
    sys.exit(2)

per = {}
for r in (1, 2, 3):
    g1 = qps(out / f"r{r}/g1/rocks_parity_bench.json")
    rocks = qps(out / f"r{r}/rocks/rocks_parity_bench.json")
    kvr = qps(out / f"r{r}/kvr/rocks_parity_bench.json")
    rkvr = qps(out / f"r{r}/rocks-kvr/rocks_parity_bench.json")
    ratios = {}
    for s in GATED:
        if s in KVR:
            c, p = kvr.get(s), rkvr.get(s)
        else:
            c, p = g1.get(s), rocks.get(s)
        if c and p and p > 0:
            ratios[s] = c / p
    per[r] = ratios

fails, mins = [], []
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
    print(f"RESULT=P11_FAIL min_ratio={min(mins) if mins else 'nan'} fail={','.join(fails)}")
    sys.exit(1)
print(f"RESULT=P11_PASS min_ratio={min(mins):.3f}")
PY
rc=$?
echo "RESULT=P11_DONE rc=$rc $(date -u +%Y-%m-%dT%H:%M:%SZ)"
mkdir -p /data
echo "rc=$rc ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)" >/data/p11-last-result.txt 2>/dev/null || true
sleep infinity
