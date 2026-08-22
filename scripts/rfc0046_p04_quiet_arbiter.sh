#!/usr/bin/env bash
# RFC-0046 P0.4 — quiet 3× re-arbiter with the NEW product default
# (HistoryHorizon::Window(24h) + archive tier, post-b68f9a1), plus the
# standing-number renewals that ride the same quiet window (RFC-0044
# async column continuity; RFC-0045 P1.3 mc50/lock_prewrite; kvrocks
# 2M long-window for the pipeline/GET straddles).
#
# Columns (peer always RocksDB DEFAULT, ROCKS_PARITY_SYNC=0 — official):
#   g1    — compat default (fdatasync before Ok), default suite (official
#           16). 0041 official column.
#   async — PEDRA_PARITY_ASYNC=1, default suite. 0044 async column
#           (E/scan/A–D + deps_lock_prewrite continuity).
#   kvr   — ROCKS_PARITY_SUITE=kvrocks both engines (suite selection, not
#           the experiment-only ROCKS_PARITY_ONLY filter), async column.
#   long  — kvrocks set/get/pipelined_set at ROCKS_YCSB_OPS=2M, async,
#           ONLY-filtered both sides = same method as the rfc0044-p2
#           long-window evidence (experiment-grade by design).
# NO floor gate here: an arbiter completes 3× and records — gating is the
# official columns script's job, not the measurement's.
#
# Gate: 1-min load < 10 (P2.1 quiet bar). Builds first (dirty OK), then
# polls every 60s and auto-fires. G1 durability regression runs first and
# IS fatal (correctness, not load-sensitive).
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

while :; do
  L=$(load1)
  echo "$(date +%H:%M:%S) load1=$L" >> "$LOG"
  ok=$(awk -v l="$L" 'BEGIN{print (l<10)?1:0}')
  [[ "$ok" == 1 ]] && break
  sleep 60
done
echo "QUIET at $(date +%H:%M:%S) load1=$(load1)" >> "$LOG"

# G1 durability regression first — fatal (typed fail-closed contract).
{
  cargo test -q -p pedradb-core --test wal_durability_adversarial 2>&1 | tail -3
  cargo test -q -p pedradb-core --lib -- g1 crash durab 2>&1 | tail -3
} >> "$LOG" || { echo "G1-REGRESSION-FAILED" >> "$LOG"; exit 3; }

compat() { # $1 out  $2 async(0/1)  $3 suite(optional)  $4 only(optional)
  local out="$1" async="$2" suite="${3:-}" only="${4:-}"
  [[ -n "$suite" ]] && export ROCKS_PARITY_SUITE="$suite"
  [[ -n "$only" ]] && export ROCKS_PARITY_ONLY="$only"
  PEDRA_PARITY_ASYNC=$async \
    cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- \
    "$out" compat
  local rc=$?
  unset ROCKS_PARITY_SUITE ROCKS_PARITY_ONLY
  return $rc
}

rocks() { # $1 out  $2 suite(optional)  $3 only(optional)
  local out="$1" suite="${2:-}" only="${3:-}"
  [[ -n "$suite" ]] && export ROCKS_PARITY_SUITE="$suite"
  [[ -n "$only" ]] && export ROCKS_PARITY_ONLY="$only"
  cargo run -q --release -p rocksdb-parity-bench --features real --bin rocks-parity-bench -- \
    "$out" rocksdb
  local rc=$?
  unset ROCKS_PARITY_SUITE ROCKS_PARITY_ONLY
  return $rc
}

for r in 1 2 3; do
  echo "=== round $r g1-compat $(date +%H:%M:%S) load1=$(load1) ===" >> "$LOG"
  compat "$OUT/r$r/g1/compat" 0 || echo "ROUND-DISCARDED r$r g1" >> "$LOG"
  rm -rf "$OUT/r$r/g1/compat/db-compat"
  echo "=== round $r async-compat $(date +%H:%M:%S) load1=$(load1) ===" >> "$LOG"
  compat "$OUT/r$r/async/compat" 1 || echo "ROUND-DISCARDED r$r async" >> "$LOG"
  rm -rf "$OUT/r$r/async/compat/db-compat"
  echo "=== round $r kvr-compat $(date +%H:%M:%S) load1=$(load1) ===" >> "$LOG"
  compat "$OUT/r$r/kvr/compat" 1 kvrocks || echo "ROUND-DISCARDED r$r kvr" >> "$LOG"
  rm -rf "$OUT/r$r/kvr/compat/db-compat"

  echo "=== round $r rocks-default $(date +%H:%M:%S) load1=$(load1) ===" >> "$LOG"
  rocks "$OUT/r$r/rocks" || echo "ROUND-DISCARDED r$r rocks" >> "$LOG"
  rm -rf "$OUT/r$r/rocks/db-rocksdb"
  echo "=== round $r rocks-kvr $(date +%H:%M:%S) load1=$(load1) ===" >> "$LOG"
  rocks "$OUT/r$r/rocks-kvr" kvrocks || echo "ROUND-DISCARDED r$r rocks-kvr" >> "$LOG"
  rm -rf "$OUT/r$r/rocks-kvr/db-rocksdb"

  ROCKS_PARITY_PEER="$OUT/r$r/rocks/rocks_parity_bench.json" \
    cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
    "$OUT/r$r/g1/compat/rocks_parity_bench.json" "$OUT/r$r/g1/compare" \
    || echo "COMPARE-FAILED r$r g1" >> "$LOG"
  ROCKS_PARITY_PEER="$OUT/r$r/rocks/rocks_parity_bench.json" \
    cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
    "$OUT/r$r/async/compat/rocks_parity_bench.json" "$OUT/r$r/async/compare" \
    || echo "COMPARE-FAILED r$r async" >> "$LOG"
  ROCKS_PARITY_PEER="$OUT/r$r/rocks-kvr/rocks_parity_bench.json" \
    cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
    "$OUT/r$r/kvr/compat/rocks_parity_bench.json" "$OUT/r$r/kvr/compare" \
    || echo "COMPARE-FAILED r$r kvr" >> "$LOG"
done

# Long window 2M (async): set/get/pipelined_set, ONLY both sides — same
# method as the rfc0044-p2 long-window straddle evidence (p11/p13).
echo "=== long 2M compat $(date +%H:%M:%S) load1=$(load1) ===" >> "$LOG"
ROCKS_YCSB_OPS=2000000 compat "$OUT/long/compat" 1 kvrocks \
  "kvrocks_set,kvrocks_get,kvrocks_pipelined_set" \
  || echo "LONG-COMPAT-FAILED" >> "$LOG"
rm -rf "$OUT/long/compat/db-compat"
echo "=== long 2M rocks $(date +%H:%M:%S) load1=$(load1) ===" >> "$LOG"
ROCKS_YCSB_OPS=2000000 rocks "$OUT/long/rocks" kvrocks \
  "kvrocks_set,kvrocks_get,kvrocks_pipelined_set" \
  || echo "LONG-ROCKS-FAILED" >> "$LOG"
rm -rf "$OUT/long/rocks/db-rocksdb"
unset ROCKS_YCSB_OPS
ROCKS_PARITY_PEER="$OUT/long/rocks/rocks_parity_bench.json" \
  cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
  "$OUT/long/compat/rocks_parity_bench.json" "$OUT/long/compare" \
  || echo "LONG-COMPARE-FAILED" >> "$LOG"

python3 - "$OUT" <<'PY' >> "$LOG"
import json, statistics, sys
from pathlib import Path
out = Path(sys.argv[1])
for col in ("g1", "async", "kvr"):
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
p = out / "long" / "compare" / "compare_report.json"
if p.exists():
    rep = json.loads(p.read_text())
    print("--- long 2M (single) ---")
    for x in rep.get("ratios") or []:
        v = x.get("compat_over_rocksdb")
        if v is not None:
            print(f"{x['shape']:24s} ratio={v:.2f}")
PY
echo "P04-ARBITER-DONE" >> "$LOG"
