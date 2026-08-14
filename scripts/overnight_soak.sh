#!/usr/bin/env bash
# RFC-0020 P0.2 — overnight / volume soak entry.
# PEDRA_SOAK_TRIALS defaults to 1000. Writes volume_report.json (arg or cwd).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
TRIALS="${PEDRA_SOAK_TRIALS:-1000}"
# Min assert floor: 1000 by default; set PEDRA_SOAK_MIN to require 10k for P1.6.
MIN="${PEDRA_SOAK_MIN:-1000}"
REPORT="${1:-$ROOT/volume_report.json}"
export PEDRA_SOAK_TRIALS="$TRIALS"
export PEDRA_SOAK_MODE="${PEDRA_SOAK_MODE:-ops}"

echo "overnight_soak PEDRA_SOAK_TRIALS=$TRIALS mode=$PEDRA_SOAK_MODE min=$MIN report=$REPORT"
cargo run -q -p pedradb-dst --bin volume_soak -- "$REPORT"

# Assert report fields
python3 - <<PY
import json, sys
from pathlib import Path
p = Path("$REPORT")
s = json.loads(p.read_text())
trials = int(s.get("trials", 0))
sw = int(s.get("silent_wrong", 1))
mn = int("$MIN")
print(json.dumps(s, indent=2))
if trials < mn:
    print(f"FAIL: trials {trials} < {mn}", file=sys.stderr)
    sys.exit(1)
if sw != 0:
    print(f"FAIL: silent_wrong={sw}", file=sys.stderr)
    sys.exit(1)
print(f"overnight_soak OK trials={trials} silent_wrong={sw}")
PY
