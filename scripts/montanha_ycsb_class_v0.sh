#!/usr/bin/env bash
# RFC-0021 P2.5 — YCSB-class v0: workload A-like (50/50 read/update) via perf gate knobs.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${1:-$ROOT/findings/ycsb-v0-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"
export MONTANHA_PERF_PUTS="${MONTANHA_PERF_PUTS:-500}"
export MONTANHA_PERF_GETS="${MONTANHA_PERF_GETS:-500}"
export MONTANHA_PERF_TX="${MONTANHA_PERF_TX:-100}"
export MONTANHA_PERF_PAYLOAD="${MONTANHA_PERF_PAYLOAD:-100}"
bash scripts/montanha_perf_gate_v0.sh "$OUT"
python3 - <<PY
import json
from pathlib import Path
out = Path("$OUT")
r = json.loads((out / "perf_report.json").read_text())
r["gate"] = "rfc0021-p2.5-ycsb-class-v0"
r["workload"] = "update+read+multi_key_tx (not full YCSB port)"
(out / "ycsb_class_report.json").write_text(json.dumps(r, indent=2) + "\n")
print("ycsb_class_v0 OK")
PY
