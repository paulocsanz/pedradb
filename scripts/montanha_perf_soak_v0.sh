#!/usr/bin/env bash
# RFC-0021 P1.6 — larger soak knobs on perf gate.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/findings/perf-soak-$(date -u +%Y%m%dT%H%M%SZ)}"
export MONTANHA_PERF_PUTS="${MONTANHA_PERF_PUTS:-2000}"
export MONTANHA_PERF_GETS="${MONTANHA_PERF_GETS:-2000}"
export MONTANHA_PERF_TX="${MONTANHA_PERF_TX:-200}"
export MONTANHA_PERF_PAYLOAD="${MONTANHA_PERF_PAYLOAD:-256}"
bash "$ROOT/scripts/montanha_perf_gate_v0.sh" "$OUT"
python3 - <<PY
import json
from pathlib import Path
p = Path("$OUT/perf_report.json")
r = json.loads(p.read_text())
r["gate"] = "rfc0021-p1.6-perf-soak-v0"
p.write_text(json.dumps(r, indent=2) + "\n")
print("perf_soak_v0 OK puts", r["put"]["n"])
PY
