#!/usr/bin/env bash
# RFC-0017 P1.4 / universe width — Montanha chaos soak (in-tree).
# Runs store FDB-path suite + multi-process smokes + optional DST gate.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
ROUNDS="${PEDRA_MONTA_ROUNDS:-3}"
LOG="${1:-$ROOT/findings/montanha-chaos-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$LOG"

echo "montanha_chaos_soak rounds=$ROUNDS → $LOG"
FAIL=0
for ((r=0; r<ROUNDS; r++)); do
  echo "== round $r ==" | tee -a "$LOG/soak.log"
  if ! cargo test -q -p pedradb-store --test montanha_fdb_path -- --nocapture \
      2>&1 | tee -a "$LOG/round-$r-fdb-path.log"; then
    FAIL=1
    echo "FAIL fdb_path round $r" | tee -a "$LOG/soak.log"
  fi
  if ! cargo test -q -p pedradb-store --test multiprocess_tx -- --nocapture \
      2>&1 | tee -a "$LOG/round-$r-mp.log"; then
    FAIL=1
    echo "FAIL multiprocess_tx round $r" | tee -a "$LOG/soak.log"
  fi
done
if ! bash scripts/montanha_scale_gate_v0.sh "$LOG/scale-gate" 2>&1 | tee -a "$LOG/scale-gate.log"; then
  FAIL=1
  echo "FAIL scale_gate" | tee -a "$LOG/soak.log"
fi

python3 - <<PY
import json, time, sys
from pathlib import Path
log = Path("$LOG")
fail = int("$FAIL")
report = {
    "gate": "montanha-chaos",
    "rounds": int("$ROUNDS"),
    "silent_wrong": fail,
    "suites": ["montanha_fdb_path", "multiprocess_tx", "scale_gate"],
    "log_dir": str(log),
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
}
(log / "montanha_chaos_report.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
if fail:
    sys.exit(1)
print("montanha_chaos_soak OK")
PY
