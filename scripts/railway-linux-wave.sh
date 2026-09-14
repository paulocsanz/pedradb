#!/bin/bash
# DIAG Linux Railway vs Rocks default (SYNC=0). Never cartaz unless
# Rocks overwrite_mc4 >= 260kQPS on a valid round.
# SUITE=ycsb: overwrite_mc4 lives in run_clients.
set -u
echo "=== RAILWAY_WAVE_START $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
uname -a
nproc
free -h || true
cat /proc/loadavg
grep -m1 'model name' /proc/cpuinfo || true
df -h /data /tmp 2>/dev/null || df -h

BIN="${BIN:-/usr/local/bin/rocks-parity-bench}"
if [ ! -x "$BIN" ]; then
  BIN=$(command -v rocks-parity-bench || true)
fi
if [ ! -x "$BIN" ]; then
  echo "RESULT=NO_BIN"
  exit 1
fi
echo "BIN=$BIN"
$BIN --help >/dev/null 2>&1 || true

OUT="${OUT:-/data/wave}"
mkdir -p "$OUT"
load1() { awk '{print $1}' /proc/loadavg; }

COMMON="ROCKS_PARITY_SYNC=0 ROCKS_PARITY_MC_FRESH=1 ROCKS_PARITY_CLIENTS=4 ROCKS_PARITY_SUITE=ycsb ROCKS_YCSB_PAYLOAD=100 ROCKS_YCSB_DIST=zipfian"

run_pair() {
  local tag=$1 only=$2 recs=$3 ops=$4
  local dest="$OUT/$tag"
  mkdir -p "$dest"
  echo "[$(date -u +%FT%TZ)] $tag rocks start load1=$(load1) mem=$(awk '/MemAvailable/{print $2}' /proc/meminfo)"
  env $COMMON ROCKS_YCSB_RECORDS=$recs ROCKS_YCSB_OPS=$ops ROCKS_PARITY_ONLY=$only \
    $BIN "$dest/rocks" rocksdb > "$dest/rocks.log" 2>&1 || echo "FAIL $tag rocks"
  echo "[$(date -u +%FT%TZ)] $tag compat start load1=$(load1)"
  env $COMMON ROCKS_YCSB_RECORDS=$recs ROCKS_YCSB_OPS=$ops ROCKS_PARITY_ONLY=$only \
    PEDRA_PARITY_ASYNC=1 \
    $BIN "$dest/compat" compat > "$dest/compat.log" 2>&1 || echo "FAIL $tag compat"
  echo "[$(date -u +%FT%TZ)] $tag done load1=$(load1)"
}

# --- canary 100k ---
for r in 1 2 3; do
  run_pair canary-ow-r$r deps_cache_overwrite_mc4 100000 100000
done

python3 - "$OUT" <<'PY'
import json, os, sys
out = sys.argv[1]
def qps(p, name):
    try:
        d = json.load(open(p))
        sync = d.get("sync")
        for b in d.get("benches", []):
            if b.get("name") == name:
                return b.get("qps"), sync
    except Exception:
        return None, None
    return None, None
ok = 0
for r in (1, 2, 3):
    c, _ = qps(f"{out}/canary-ow-r{r}/compat/rocks_parity_bench.json", "deps_cache_overwrite_mc4")
    k, ks = qps(f"{out}/canary-ow-r{r}/rocks/rocks_parity_bench.json", "deps_cache_overwrite_mc4")
    ratio = (c / k) if c and k else None
    valid = bool(k and k >= 260000 and ks is False)
    if valid:
        ok += 1
    print(f"CANARY_OW r{r} pedra={c} rocks={k} sync={ks} ratio={ratio} valid={valid}")
print(f"CANARY_VALID {ok}/3")
open(f"{out}/canary.valid", "w").write(str(ok))
PY

VALID=$(cat "$OUT/canary.valid" 2>/dev/null || echo 0)
if [ "$VALID" -lt 1 ]; then
  echo "HOST_NOISY canary Rocks overwrite never >=260k — skip 25M"
  echo "RAILWAY_WAVE_DONE noisy"
  exit 0
fi

# --- 25M overwrite (bounded vs 3GiB WARM floor even if the box has more RAM) ---
for r in 1 2 3; do
  run_pair ow25-r$r deps_cache_overwrite_mc4 25000000 200000
done
# --- ycsb_f_mc4 at 100k (same as Darwin DIAG cell) ---
for r in 1 2 3; do
  run_pair yf-r$r ycsb_f_mc4 100000 100000
done

python3 - "$OUT" <<'PY'
import json, os, sys
out = sys.argv[1]
def qps(p, name):
    try:
        d = json.load(open(p))
        for b in d.get("benches", []):
            if b.get("name") == name:
                return b.get("qps"), d.get("sync")
    except Exception:
        return None, None
    return None, None
def report(tag, name, n=3):
    rounds = []
    for r in range(1, n+1):
        c, _ = qps(f"{out}/{tag}-r{r}/compat/rocks_parity_bench.json", name)
        k, ks = qps(f"{out}/{tag}-r{r}/rocks/rocks_parity_bench.json", name)
        if c and k:
            print(f"{tag} r{r}: ratio={c/k:.4f} pedra={c:.0f} rocks={k:.0f} sync={ks}")
            rounds.append(c/k)
    if rounds:
        print(f"{tag}_RESULT min={min(rounds):.4f} med={sorted(rounds)[len(rounds)//2]:.4f}")
report("ow25", "deps_cache_overwrite_mc4")
report("yf", "ycsb_f_mc4")
PY
echo "RAILWAY_WAVE_DONE $(date -u +%Y-%m-%dT%H:%M:%SZ)"
