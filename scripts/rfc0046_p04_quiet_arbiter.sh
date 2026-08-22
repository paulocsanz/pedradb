#!/usr/bin/env bash
# RFC-0046 P0.4 — quiet 3× re-arbiter with the NEW product default
# (HistoryHorizon::Window(24h) + archive tier, post-b68f9a1).
#
# What it proves: official 0041 columns measured on the new default —
#   g1    — compat default (fdatasync before Ok) vs RocksDB default
#           (sync=false), floor 2.0 (RFC-0041 product floor, all shapes)
#   async — PEDRA_PARITY_ASYNC=1 vs RocksDB default (RFC-0044 column,
#           E/scan/A–D continuity with the 04c7aa2 arbiter)
# plus a G1 durability regression pass before the rounds (the retention
# default must not touch the WAL contract).
#
# Gate: 1-min load < 10 (the P2.1 quiet bar). Builds first (dirty OK),
# then polls every 60s and auto-fires when quiet. Load logged per phase;
# rounds that started ≥ 10 are dirty by definition — discard at read.
# Peer guard: ROCKS_PARITY_SYNC=0; compare exits 2 on a sync=true peer.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
OUT="findings/rfc0046-p04"
mkdir -p "$OUT"
LOG="$OUT/loads.txt"
export ROCKS_PARITY_SYNC=0

load1() { sysctl -n vm.loadavg | tr ',' '.' | awk '{print $2}'; }

echo "=== build start $(date +%H:%M:%S) load1=$(load1) ===" >> "$LOG"
cargo build -q --release -p rocksdb-parity-bench --bin rocks-parity-bench --bin rocks-parity-compare \
  || { echo "BUILD-FAILED" >> "$LOG"; exit 1; }
cargo build -q --release -p rocksdb-parity-bench --features real --bin rocks-parity-bench \
  || { echo "BUILD-FAILED-REAL" >> "$LOG"; exit 1; }

# Gate: quiet = 1-min load < 10
while :; do
  L=$(load1)
  echo "$(date +%H:%M:%S) load1=$L" >> "$LOG"
  ok=$(awk -v l="$L" 'BEGIN{print (l<10)?1:0}')
  [[ "$ok" == 1 ]] && break
  sleep 60
done
echo "QUIET at $(date +%H:%M:%S) load1=$(load1)" >> "$LOG"

# G1 durability regression first (typed fail-closed contract, new default)
{
  cargo test -q -p pedradb-core --test wal_durability_adversarial 2>&1 | tail -3
  cargo test -q -p pedradb-core --lib -- g1 crash durab 2>&1 | tail -3
} >> "$LOG" || { echo "G1-REGRESSION-FAILED" >> "$LOG"; exit 3; }

for r in 1 2 3; do
  echo "=== round $r g1-compat $(date +%H:%M:%S) load1=$(load1) ===" >> "$LOG"
  cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- \
    "$OUT/r$r/g1/compat" compat || exit 4
  rm -rf "$OUT/r$r/g1/compat/db-compat"
  echo "=== round $r async-compat $(date +%H:%M:%S) load1=$(load1) ===" >> "$LOG"
  PEDRA_PARITY_ASYNC=1 \
  cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- \
    "$OUT/r$r/async/compat" compat || exit 4
  rm -rf "$OUT/r$r/async/compat/db-compat"
  echo "=== round $r rocks $(date +%H:%M:%S) load1=$(load1) ===" >> "$LOG"
  cargo run -q --release -p rocksdb-parity-bench --features real --bin rocks-parity-bench -- \
    "$OUT/r$r/rocks" rocksdb || exit 4
  rm -rf "$OUT/r$r/rocks/db-rocksdb"
  ROCKS_PARITY_PEER="$OUT/r$r/rocks/rocks_parity_bench.json" \
  ROCKS_PARITY_RATIO_FLOOR=2.0 \
    cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
    "$OUT/r$r/g1/compat/rocks_parity_bench.json" "$OUT/r$r/g1/compare" || exit 2
  ROCKS_PARITY_PEER="$OUT/r$r/rocks/rocks_parity_bench.json" \
    cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
    "$OUT/r$r/async/compat/rocks_parity_bench.json" "$OUT/r$r/async/compare"
done

python3 - "$OUT" <<'PY' >> "$LOG"
import json, statistics, sys
from pathlib import Path
out = Path(sys.argv[1])
for col in ("g1", "async"):
    per = {}
    for r in (1, 2, 3):
        p = out / f"r{r}" / col / "compare" / "compare_report.json"
        if not p.exists():
            continue
        rep = json.loads(p.read_text())
        for x in rep.get("ratios") or []:
            v = x.get("compat_over_rocksdb")
            if v is not None:
                per.setdefault(x["shape"], []).append(v)
    print(f"--- {col} median-of-3 ---")
    for s, vs in sorted(per.items(), key=lambda kv: -statistics.median(kv[1])):
        print(f"{s:24s} med={statistics.median(vs):.2f} "
              f"runs={'/'.join(f'{v:.2f}' for v in vs)}")
PY
echo "P04-ARBITER-DONE" >> "$LOG"
