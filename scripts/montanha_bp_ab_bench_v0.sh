#!/usr/bin/env bash
# Lab A/B on the heavier fdb-bench core suite: off vs Pedra L0 write backpressure.
# Ratios are observational (no field-FDB claim; CI does not enforce thr floor/ceiling).
#
# Usage:
#   scripts/montanha_bp_ab_bench_v0.sh [out_dir]
# Env (both runs):
#   MONTANHA_BENCH_N        ops per bench (default 50; CI can lower)
#   MONTANHA_BENCH_WARMUP   warmup ops (default 4)
#   MONTANHA_BENCH_PAYLOAD  value bytes (default 64)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OUT="${1:-$ROOT/findings/bp-ab-bench-$(date -u +%Y%m%dT%H%M%SZ)}"
OFF_DIR="$OUT/off"
BP_DIR="$OUT/bp"
mkdir -p "$OFF_DIR" "$BP_DIR"

export MONTANHA_BENCH_SUITE="${MONTANHA_BENCH_SUITE:-core}"
export MONTANHA_BENCH_N="${MONTANHA_BENCH_N:-50}"
export MONTANHA_BENCH_WARMUP="${MONTANHA_BENCH_WARMUP:-4}"
export MONTANHA_BENCH_PAYLOAD="${MONTANHA_BENCH_PAYLOAD:-64}"

echo "montanha_bp_ab_bench_v0 → $OUT suite=$MONTANHA_BENCH_SUITE n=$MONTANHA_BENCH_N bp=off"
env -u MONTANHA_WRITE_BACKPRESSURE \
  cargo run -q -p pedradb-store --release --bin montanha-fdb-bench -- "$OFF_DIR"

echo "montanha_bp_ab_bench_v0 → $OUT suite=$MONTANHA_BENCH_SUITE n=$MONTANHA_BENCH_N bp=1"
MONTANHA_WRITE_BACKPRESSURE=1 \
  cargo run -q -p pedradb-store --release --bin montanha-fdb-bench -- "$BP_DIR"

OFF_JSON="$OFF_DIR/fdb_shaped_bench.json"
BP_JSON="$BP_DIR/fdb_shaped_bench.json"
test -f "$OFF_JSON"
test -f "$BP_JSON"

python3 - <<PY
import json
from pathlib import Path

off = json.loads(Path("$OFF_JSON").read_text())
bp = json.loads(Path("$BP_JSON").read_text())
assert off.get("write_backpressure") is False, off.get("write_backpressure")
assert bp.get("write_backpressure") is True, bp.get("write_backpressure")

def by_name(r):
    out = {}
    for b in r.get("benches") or []:
        name = b.get("name")
        q = b.get("keys_per_s", b.get("qps"))
        if name and q is not None:
            out[name] = b
    return out

off_b = by_name(off)
bp_b = by_name(bp)
shared = sorted(set(off_b) & set(bp_b))
assert shared, "no shared bench names between runs"

rows = []
for name in shared:
    a = float(off_b[name].get("keys_per_s", off_b[name].get("qps")))
    b = float(bp_b[name].get("keys_per_s", bp_b[name].get("qps")))
    ratio = round(b / a, 4) if a > 0 else None
    rows.append({
        "name": name,
        "off_qps": a,
        "bp_qps": b,
        "bp_over_off": ratio,
    })

adm_off = off.get("admission_core_b") or off.get("admission_core_a") or {}
adm_bp = bp.get("admission_core_b") or bp.get("admission_core_a") or {}

report = {
    "ab": "montanha-write-backpressure-fdb-bench-core-v0",
    "honesty": "Lab laptop in-process core suite. Not field FDB. Ratios observational; no thr threshold enforced.",
    "suite": "$MONTANHA_BENCH_SUITE",
    "n": $MONTANHA_BENCH_N,
    "payload_bytes": $MONTANHA_BENCH_PAYLOAD,
    "off_path": "$OFF_JSON",
    "bp_path": "$BP_JSON",
    "ratios": rows,
    "admission_off": adm_off,
    "admission_bp": adm_bp,
    "admission_delta": {
        "l0_files_max_off": adm_off.get("l0_files_max"),
        "l0_files_max_bp": adm_bp.get("l0_files_max"),
        "write_stall_count_sum_off": adm_off.get("write_stall_count_sum"),
        "write_stall_count_sum_bp": adm_bp.get("write_stall_count_sum"),
        "write_pressure_count_sum_off": adm_off.get("write_pressure_count_sum"),
        "write_pressure_count_sum_bp": adm_bp.get("write_pressure_count_sum"),
        "write_stall_l0_bp": adm_bp.get("write_stall_l0"),
        "write_pressure_l0_bp": adm_bp.get("write_pressure_l0"),
    },
    "pass": True,
}
out = Path("$OUT") / "bp_ab_bench_report.json"
out.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
print("wrote", out)

# Config must be live under BP and off in the unconstrained run.
assert int(adm_bp.get("write_stall_l0") or 0) > 0, adm_bp
assert int(adm_bp.get("write_pressure_l0") or 0) > 0, adm_bp
assert int(adm_off.get("write_stall_l0") or 0) == 0, adm_off
print("bp_ab bench v0 OK benches=", len(rows))
PY
