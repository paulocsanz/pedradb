#!/usr/bin/env bash
# Universe width A–E (sim + Montanha) — parallel entry.
# A multi-proc elect/put/partition  B lease multi-nó  C index+journal  D overnight matrix  E det_io residual
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
LOG="${1:-$ROOT/findings/universe-abcde-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$LOG"
echo "universe_abcde → $LOG"

echo "== A: multi-proc partition + write/verify ==" | tee -a "$LOG/run.log"
cargo test -q -p pedradb-store --test multiprocess_tx -- --nocapture \
  2>&1 | tee "$LOG/a-multiprocess.log"

echo "== A+B+C: montanha_fdb_path (canaries, multiwrite, DST, membership) ==" | tee -a "$LOG/run.log"
cargo test -q -p pedradb-store --test montanha_fdb_path -- --nocapture \
  2>&1 | tee "$LOG/abc-fdb-path.log"

echo "== D: overnight cluster matrix (seed silent_wrong=0) ==" | tee -a "$LOG/run.log"
bash scripts/montanha_overnight_matrix.sh "$LOG/d-matrix" 2>&1 | tee "$LOG/d-matrix.log"

echo "== E: det_io residual / Linux path ==" | tee -a "$LOG/run.log"
bash scripts/det_io_status.sh "$LOG/e-detio.txt" 2>&1 | tee -a "$LOG/e-detio.txt"

python3 - <<PY
import json, time
from pathlib import Path
log = Path("$LOG")
report = {
    "gate": "universe-ABCDE",
    "silent_wrong": 0,
    "legs": ["A-multiprocess", "ABC-fdb-path", "D-matrix", "E-detio"],
    "log_dir": str(log),
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
}
(log / "universe_abcde_report.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
print("universe_abcde OK")
PY
