#!/bin/sh
# RFC-0185 P0.3 — Linux cartaz: deps_cache_overwrite_mc4 min of 3 quiet rounds
# > 1.0 vs Rocks default (WriteOptions.sync=false). Column A: compat async
# (PEDRA_PARITY_ASYNC=1), 4 clients, fresh DB per run, isolated rounds.
# Collapsed Rocks (<=157k) is a peer anomaly, never a Pedra win.
echo "=== P03_START $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
uname -a
grep -m1 'model name' /proc/cpuinfo || true
nproc
cd /src/pedradb || { echo "RESULT=BOOTSTRAP_FAIL"; sleep infinity; }
export CARGO_TARGET_DIR=/tmp/p03-target
export CARGO_PROFILE_RELEASE_DEBUG=0
echo "--- build offline (j3) started $(date -u +%H:%M:%S) ---"
cargo build --release --offline -p rocksdb-parity-bench --features real -j 3 \
  || cargo build --release -p rocksdb-parity-bench --features real -j 3 \
  || { echo "RESULT=BUILD_FAIL"; sleep infinity; }
cp "$CARGO_TARGET_DIR/release/rocks-parity-bench" /tmp/rocks-parity-bench
cp "$CARGO_TARGET_DIR/release/rocks-parity-compare" /tmp/rocks-parity-compare
rm -rf "$CARGO_TARGET_DIR"
echo "RESULT=BUILD_OK $(date -u +%H:%M:%S)"
BIN=/tmp/rocks-parity-bench
CMP=/tmp/rocks-parity-compare
OUT=/tmp/p03
rm -rf "$OUT"; mkdir -p "$OUT"

load1() { awk '{print $1}' /proc/loadavg; }
quiet_gate() {
  c=0
  while [ "$c" -lt 2 ]; do
    l=$(load1)
    if awk -v l="$l" 'BEGIN{exit !(l < 2)}'; then c=$((c + 1)); else c=0; fi
    echo "gate load1=${l} (${c}/2)"
    if [ "$c" -lt 2 ]; then sleep 20; fi
  done
  echo "QUIET load1=$(load1) $(date -u +%H:%M:%S)"
}
quiet_gate

export ROCKS_PARITY_SYNC=0
export ROCKS_PARITY_CLIENTS=4
# P0.11 isolated recipe: ycsb suite seeds 1024 keys, run_clients emits
# ycsb_a_mc4 / ycsb_f_mc4 / deps_cache_overwrite_mc4. Bin rm's the db dir
# per run (fresh DB); ONLY skips the 1c shapes.
export ROCKS_PARITY_SUITE=ycsb
export ROCKS_PARITY_ONLY=deps_cache_overwrite_mc4
export ROCKS_YCSB_RECORDS=1024 ROCKS_YCSB_OPS=10000 ROCKS_YCSB_PAYLOAD=100
export ROCKS_YCSB_DIST=uniform
export ROCKS_DEPS_BATCH=32
export ROCKS_PARITY_BIG=0

for r in 1 2 3; do
  echo "=== round $r compat-async $(date -u +%H:%M:%S) load1=$(load1) ==="
  rm -rf "$OUT/r$r/compat" "$OUT/r$r/rocks"
  PEDRA_PARITY_ASYNC=1 "$BIN" "$OUT/r$r/compat" compat || echo "RUN-FAIL compat r$r"
  echo "=== round $r rocks-default $(date -u +%H:%M:%S) load1=$(load1) ==="
  ROCKS_PARITY_SYNC=0 "$BIN" "$OUT/r$r/rocks" rocksdb || echo "RUN-FAIL rocks r$r"
  ROCKS_PARITY_PEER="$OUT/r$r/rocks/rocks_parity_bench.json" "$CMP" \
    "$OUT/r$r/compat/rocks_parity_bench.json" "$OUT/r$r/compare" \
    || echo "COMPARE-FAILED r$r"
  rm -rf "$OUT/r$r/compat/db-compat" "$OUT/r$r/rocks/db-rocksdb"
done

python3 - "$OUT" <<'PY'
import json, sys
from pathlib import Path
out = Path(sys.argv[1])
SHAPE = "deps_cache_overwrite_mc4"
COLLAPSED = 157_000.0
QUIET = 260_000.0

def bench(path):
    d = json.loads(Path(path).read_text())
    row = next((b for b in d.get("benches", []) if b.get("name") == SHAPE), None)
    return d, row

print("P03_COLUNA_A shape=deps_cache_overwrite_mc4 rounds=3 peer=rocks-default(sync=false) compat=PEDRA_PARITY_ASYNC=1 clients=4 records=1024 ops=10000 uniform")
ratios, quiet_ok, sync_ok = [], True, True
for r in (1, 2, 3):
    cd, crow = bench(out / f"r{r}/compat/rocks_parity_bench.json")
    rd, rrow = bench(out / f"r{r}/rocks/rocks_parity_bench.json")
    if crow is None or rrow is None:
        print(f"  r{r} MISSING compat={crow is not None} rocks={rrow is not None}")
        continue
    p, q = crow["qps"], rrow["qps"]
    if cd.get("sync") is True:
        sync_ok = False
        print(f"  r{r} compat sync:true — NOT column A")
    if rd.get("sync") is not False:
        sync_ok = False
        print(f"  r{r} peer sync:{rd.get('sync')} — refused (official peer is sync=false)")
    ratio = p / q if q and q > 0 else 0.0
    ratios.append(ratio)
    band = "quiet" if q > COLLAPSED else "COLLAPSED"
    print(f"  r{r} ratio={ratio:.3f} pedra_qps={p:.0f} rocks_qps={q:.0f} rocks_band={band}")
    if q <= COLLAPSED:
        quiet_ok = False

if len(ratios) < 3:
    print("RESULT=P03_FAIL incomplete_rounds")
    sys.exit(1)
mn = min(ratios)
print(f"  min={mn:.3f} rounds={[round(x, 3) for x in ratios]}")
if not sync_ok:
    print("RESULT=P03_FAIL sync_contract")
    sys.exit(1)
if not quiet_ok:
    print(f"RESULT=P03_PEER_ANOMALY collapsed_rocks (quiet band >{QUIET:.0f}; not a Pedra win)")
    sys.exit(2)
print(f"RESULT=P03_{'PASS' if mn > 1.0 else 'FAIL'} min={mn:.3f}")
sys.exit(0 if mn > 1.0 else 1)
PY
rc=$?
echo "RESULT=P03_DONE rc=$rc $(date -u +%Y-%m-%dT%H:%M:%SZ)"
mkdir -p /data
echo "rc=$rc ts=$(date -u +%Y-%m-%dT%H:%M:%S)" >/data/p03-last-result.txt 2>/dev/null || true
sleep infinity
