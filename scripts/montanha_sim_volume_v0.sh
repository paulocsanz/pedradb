#!/usr/bin/env bash
# RFC-0021 P0.4 — simulation volume v0 (seed bank + multi-fault suite).
#
# Default: dense seed bank × suite (minutes). For long runs:
#   PEDRA_SIM_VOLUME_ROUNDS=50 PEDRA_SIM_VOLUME_SEEDS=... ./scripts/montanha_sim_volume_v0.sh
# Optional wall budget (seconds); stops after budget with report (0 = no wall cap):
#   PEDRA_SIM_VOLUME_WALL_SECS=28800  # 8h
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
LOG="${1:-$ROOT/findings/sim-volume-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$LOG"
# 50+ seeds for bank; override with PEDRA_SIM_VOLUME_SEEDS
SEEDS="${PEDRA_SIM_VOLUME_SEEDS:-0x17CAD571,0x17CA5EED,0x17CAAF01,0x00200A17,0x0020C111,0xA11CE001,0xBEEF0001,0xCAFE0002,0xDEAD0003,0xF00D0004,0x11110001,0x22220002,0x33330003,0x44440004,0x55550005,0x66660006,0x77770007,0x88880008,0x99990009,0xAAAA000A,0xBBBB000B,0xCCCC000C,0xDDDD000D,0xEEEE000E,0xFFFF000F,0x01010101,0x02020202,0x03030303,0x04040404,0x05050505,0x10101010,0x20202020,0x30303030,0x40404040,0x50505050,0x60606060,0x70707070,0x80808080,0x90909090,0xA0A0A0A0,0xB0B0B0B0,0xC0C0C0C0,0xD0D0D0D0,0xE0E0E0E0,0xF0F0F0F0,0x12345678,0x87654321,0xABCDEF01,0xFEDCBA98,0x13579BDF}"
ROUNDS="${PEDRA_SIM_VOLUME_ROUNDS:-2}"
WALL="${PEDRA_SIM_VOLUME_WALL_SECS:-0}"
START=$(date +%s)
FAIL=0
RUNS=0
IFS=',' read -ra SEED_ARR <<< "$SEEDS"
echo "montanha_sim_volume_v0 seeds=${#SEED_ARR[@]} rounds=$ROUNDS wall=${WALL}s → $LOG"

for ((r=0; r<ROUNDS; r++)); do
  for seed in "${SEED_ARR[@]}"; do
    if [[ "$WALL" != "0" ]]; then
      now=$(date +%s)
      if (( now - START >= WALL )); then
        echo "wall budget reached after ${RUNS} runs" | tee -a "$LOG/run.log"
        break 2
      fi
    fi
    RUNS=$((RUNS + 1))
    echo "== run $RUNS seed=$seed round=$r ==" | tee -a "$LOG/run.log"
    # Full fdb_path suite (p21 multi-fault, DST, membership, TX)
    if ! cargo test -q -p pedradb-store --test montanha_fdb_path -- --nocapture \
        2>&1 | tee "$LOG/run-${RUNS}-seed-${seed}.log"; then
      FAIL=1
      echo "FAIL run $RUNS seed=$seed" | tee -a "$LOG/run.log"
    fi
  done
done

# Always include multiproc TX + chaos once
if ! cargo test -q -p pedradb-store --test multiprocess_tx -- --nocapture \
    2>&1 | tee "$LOG/multiprocess.log"; then
  FAIL=1
fi
if ! bash scripts/montanha_chaos_soak.sh "$LOG/chaos" 2>&1 | tee "$LOG/chaos.log"; then
  FAIL=1
fi

END=$(date +%s)
python3 - <<PY
import json, time
from pathlib import Path
log = Path("$LOG")
fail = int("$FAIL")
report = {
    "gate": "rfc0021-p0.4-sim-volume-v0",
    "seed_count": int("${#SEED_ARR[@]}"),
    "rounds": int("$ROUNDS"),
    "runs": int("$RUNS"),
    "wall_secs": int("$END") - int("$START"),
    "wall_budget_secs": int("$WALL"),
    "silent_wrong": fail,
    "log_dir": str(log),
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "note": "seed bank × montanha_fdb_path multi-fault; set PEDRA_SIM_VOLUME_WALL_SECS=28800 for 8h",
}
(log / "sim_volume_report.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
if fail:
    raise SystemExit(1)
print("sim_volume_v0 OK silent_wrong=0")
PY
