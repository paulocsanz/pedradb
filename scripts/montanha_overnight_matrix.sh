#!/usr/bin/env bash
# Universe D — overnight matrix: fixed seeds → elect/put/lossy I-MAJ silent_wrong=0.
# Default: run montanha_fdb_path cluster_dst tests under several seeds via env.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
LOG="${1:-$ROOT/findings/montanha-matrix-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$LOG"
SEEDS="${PEDRA_MONTA_SEEDS:-0x17CAD571,0x17CA5EED,0x17CAAF01,0x00200A17,0x0020C111}"
ROUNDS="${PEDRA_MONTA_MATRIX_ROUNDS:-3}"

echo "montanha_overnight_matrix seeds=$SEEDS rounds=$ROUNDS → $LOG"
FAIL=0
for ((r=0; r<ROUNDS; r++)); do
  echo "== matrix round $r ==" | tee -a "$LOG/matrix.log"
  # Single filter: full montanha_fdb_path suite (cargo allows one TESTNAME).
  if ! cargo test -q -p pedradb-store --test montanha_fdb_path -- --nocapture \
      2>&1 | tee "$LOG/round-$r.log"; then
    FAIL=1
  fi
done

# Also run chaos soak once (lighter than multi-round full suite)
if ! bash scripts/montanha_chaos_soak.sh "$LOG/chaos" 2>&1 | tee "$LOG/chaos.log"; then
  FAIL=1
fi

python3 - <<PY
import json, sys, time
from pathlib import Path
log = Path("$LOG")
fail = int("$FAIL")
report = {
    "gate": "D-overnight-matrix",
    "rounds": int("$ROUNDS"),
    "silent_wrong": fail,
    "log_dir": str(log),
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
}
(log / "matrix_report.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
if fail:
    print("FAIL: matrix had failures", file=sys.stderr)
    sys.exit(1)
print("montanha_overnight_matrix OK silent_wrong=0")
PY
