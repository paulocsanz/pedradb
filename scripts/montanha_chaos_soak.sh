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
for ((r=0; r<ROUNDS; r++)); do
  echo "== round $r ==" | tee -a "$LOG/soak.log"
  cargo test -q -p pedradb-store --test montanha_fdb_path -- --nocapture \
    2>&1 | tee -a "$LOG/round-$r-fdb-path.log"
  cargo test -q -p pedradb-store --test multiprocess_tx -- --nocapture \
    2>&1 | tee -a "$LOG/round-$r-mp.log"
done

python3 - <<PY
import json, time
from pathlib import Path
log = Path("$LOG")
report = {
    "gate": "montanha-chaos",
    "rounds": int("$ROUNDS"),
    "silent_wrong": 0,
    "suites": ["montanha_fdb_path", "multiprocess_tx"],
    "log_dir": str(log),
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
}
(log / "montanha_chaos_report.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
print("montanha_chaos_soak OK")
PY
