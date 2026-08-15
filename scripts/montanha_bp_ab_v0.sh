#!/usr/bin/env bash
# Lab A/B: scale-gate without vs with Pedra L0 write backpressure.
# Honest numbers only — no field-FDB claim; thr ratio is observational (not a fail threshold).
#
# Usage:
#   scripts/montanha_bp_ab_v0.sh [out_dir]
#     → runs both gates under out_dir/{off,bp} and writes out_dir/bp_ab_report.json
#   scripts/montanha_bp_ab_v0.sh --from <off_dir> <bp_dir> [out_dir]
#     → compare existing scale_report.json files (CI dual scale-gate path)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ "${1:-}" == "--from" ]]; then
  OFF_DIR="${2:?off scale-gate dir}"
  BP_DIR="${3:?bp scale-gate dir}"
  OUT="${4:-$ROOT/findings/bp-ab-$(date -u +%Y%m%dT%H%M%SZ)}"
else
  OUT="${1:-$ROOT/findings/bp-ab-$(date -u +%Y%m%dT%H%M%SZ)}"
  OFF_DIR="$OUT/off"
  BP_DIR="$OUT/bp"
  mkdir -p "$OFF_DIR" "$BP_DIR"
  echo "montanha_bp_ab_v0: run scale-gate OFF → $OFF_DIR"
  env -u MONTANHA_WRITE_BACKPRESSURE bash scripts/montanha_scale_gate_v0.sh "$OFF_DIR"
  echo "montanha_bp_ab_v0: run scale-gate BP → $BP_DIR"
  MONTANHA_WRITE_BACKPRESSURE=1 bash scripts/montanha_scale_gate_v0.sh "$BP_DIR"
fi

mkdir -p "$OUT"
OFF_JSON="$OFF_DIR/scale_report.json"
BP_JSON="$BP_DIR/scale_report.json"
test -f "$OFF_JSON"
test -f "$BP_JSON"

python3 - <<PY
import json
from pathlib import Path

off = json.loads(Path("$OFF_JSON").read_text())
bp = json.loads(Path("$BP_JSON").read_text())
assert off.get("pass") is True, off.get("failures")
assert bp.get("pass") is True, bp.get("failures")
assert off.get("write_backpressure") is False, off
assert bp.get("write_backpressure") is True, bp
for label, r in (("off", off), ("bp", bp)):
    assert "admission_r4" in r, (label, r.keys())

def kps(r, key):
    v = r.get(key)
    return float(v) if v is not None else None

off_mr = kps(off, "multi_range_keys_per_s")
bp_mr = kps(bp, "multi_range_keys_per_s")
off_sr = kps(off, "single_range_keys_per_s")
bp_sr = kps(bp, "single_range_keys_per_s")

def ratio(a, b):
    if a is None or b is None or a <= 0:
        return None
    return round(b / a, 4)

adm_off = off.get("admission_r4") or {}
adm_bp = bp.get("admission_r4") or {}

report = {
    "ab": "montanha-write-backpressure-scale-v0",
    "honesty": "Lab in-process scale-gate only. Not field FDB. thr ratio is observational; CI does not enforce thr floor/ceiling.",
    "off_path": "$OFF_JSON",
    "bp_path": "$BP_JSON",
    "off": {
        "multi_range_keys_per_s": off_mr,
        "single_range_keys_per_s": off_sr,
        "admission_r4": adm_off,
        "admission_r1": off.get("admission_r1"),
    },
    "bp": {
        "multi_range_keys_per_s": bp_mr,
        "single_range_keys_per_s": bp_sr,
        "admission_r4": adm_bp,
        "admission_r1": bp.get("admission_r1"),
    },
    "ratios": {
        "multi_range_keys_per_s_bp_over_off": ratio(off_mr, bp_mr),
        "single_range_keys_per_s_bp_over_off": ratio(off_sr, bp_sr),
    },
    "admission_delta_r4": {
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
out = Path("$OUT") / "bp_ab_report.json"
out.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
print("wrote", out)
# Config must be live under BP; thr is not gated.
assert int(adm_bp.get("write_stall_l0") or 0) > 0, adm_bp
assert int(adm_bp.get("write_pressure_l0") or 0) > 0, adm_bp
assert int(adm_off.get("write_stall_l0") or 0) == 0, adm_off
print(
    "bp_ab v0 OK mr_ratio=",
    report["ratios"]["multi_range_keys_per_s_bp_over_off"],
    "stall_l0_bp=",
    adm_bp.get("write_stall_l0"),
)
PY
