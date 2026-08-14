#!/usr/bin/env bash
# RFC-0020 P2.2 — stretch soak config (≥100k when hardware allows).
# Default stretch trials: 100000 in ops mode (fast enough for overnight box).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
TRIALS="${PEDRA_SOAK_TRIALS:-100000}"
REPORT="${1:-$ROOT/volume_report_stretch.json}"
export PEDRA_SOAK_TRIALS="$TRIALS"
export PEDRA_SOAK_MODE="${PEDRA_SOAK_MODE:-ops}"

echo "stretch_soak trials=$TRIALS mode=$PEDRA_SOAK_MODE → $REPORT"
cargo run -q -p pedradb-dst --bin volume_soak -- "$REPORT"
python3 - <<PY
import json, sys
from pathlib import Path
s = json.loads(Path("$REPORT").read_text())
print(json.dumps(s, indent=2))
if int(s.get("silent_wrong", 1)) != 0:
    sys.exit(1)
if int(s.get("trials", 0)) < 1000:
    sys.exit(1)
print("stretch_soak OK")
PY
