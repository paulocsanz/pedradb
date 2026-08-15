#!/usr/bin/env bash
# RFC-0021 / 0025 — scale option-A gate v0 (in-process, CI-friendly).
# Fails if leader diversity / multi-range put / put_batch hygiene regresses.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${1:-$ROOT/findings/scale-gate-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"
export MONTANHA_SCALE_KEYS="${MONTANHA_SCALE_KEYS:-8}"
export MONTANHA_SCALE_BATCH="${MONTANHA_SCALE_BATCH:-8}"
# Optional: MONTANHA_WRITE_BACKPRESSURE=1 for Pedra L0 admission (default off).
echo "montanha_scale_gate_v0 → $OUT write_backpressure=${MONTANHA_WRITE_BACKPRESSURE:-0}"
cargo run -q -p pedradb-store --release --bin montanha-scale-gate -- "$OUT"
test -f "$OUT/scale_report.json"
python3 - <<PY
import json
from pathlib import Path
r = json.loads(Path("$OUT/scale_report.json").read_text())
assert r.get("gate") == "rfc0021-0025-scale-option-a-v0"
assert r.get("pass") is True, r.get("failures")
assert r.get("leader_nodes_r4", 0) >= 2, r
assert r.get("multi_range_puts_ok", 0) >= 4, r
assert "write_backpressure" in r, r
assert "admission_r4" in r and "admission_r1" in r, r
want_bp = __import__("os").environ.get("MONTANHA_WRITE_BACKPRESSURE") == "1"
assert r.get("write_backpressure") is want_bp, r
adm = r.get("admission_r4") or {}
if want_bp:
    notes = " ".join(r.get("notes") or [])
    assert "l0_lim=8" in notes or "l0_lim=" in notes, notes
    assert int(adm.get("write_stall_l0") or 0) > 0, adm
print(
    "scale gate v0 OK leaders=", r["leader_nodes_r4"],
    "mr_kps=", r["multi_range_keys_per_s"],
    "write_backpressure=", r.get("write_backpressure"),
    "admission_r4=", adm,
)
PY
