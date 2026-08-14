#!/usr/bin/env bash
# RFC-0021 P0.3 — perf gate v0. Fails if no machine-readable artifact.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${1:-$ROOT/findings/perf-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"
export MONTANHA_PERF_PUTS="${MONTANHA_PERF_PUTS:-200}"
export MONTANHA_PERF_GETS="${MONTANHA_PERF_GETS:-200}"
export MONTANHA_PERF_TX="${MONTANHA_PERF_TX:-50}"
export MONTANHA_PERF_PAYLOAD="${MONTANHA_PERF_PAYLOAD:-64}"
echo "montanha_perf_gate_v0 → $OUT"
cargo run -q -p pedradb-store --bin montanha-perf-gate -- "$OUT"
test -f "$OUT/perf_report.json"
python3 - <<PY
import json
from pathlib import Path
r = json.loads(Path("$OUT/perf_report.json").read_text())
assert r.get("gate") == "rfc0021-p0.3-perf-v0"
assert r["put"]["n"] > 0 and r["put"]["qps"] > 0
assert "p99_ms" in r["put"] and "p99_ms" in r["commit_tx_pending"]
print("perf gate v0 OK", r["put"]["qps"], "put_qps", r["commit_tx_pending"]["p99_ms"], "tx_p99_ms")
PY
