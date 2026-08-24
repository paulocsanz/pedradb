#!/usr/bin/env python3
"""RFC-0054 P2.2: regression gate over a quiet battery directory.

Reads the battery layout (<out>/r{N}/{async,kvr,rocks,rocks-kvr}/rocks_parity_bench.json)
and enforces the closed-shape floors from the RFC scoreboard:

  * FLOOR-2 shapes (median round ratio >= 2.0): ycsb_a..f, deps_cache_overwrite,
    deps_lock_prewrite, deps_mvcc_latest, kvrocks_get, kvrocks_set,
    kvrocks_scan, kvrocks_pipelined_set
  * deps_apply_batch: 3/3 rounds >= 2.0 (P1.4 criterion)
  * deps_raftlog: median round ratio > 1.0 (P0.2 floor; median, not min —
    a single documented OS-stall round (rearm11 r2) must not fail the gate,
    closure evidence still uses min of 3)
  * NOT gated (open / user-deprioritized 2026-08-24): deps_scan,
    kvrocks_blob_set, kvrocks_set_mc50

Exit 0 = pass, 1 = regression (any closed shape below floor). Works for both
the macOS rearm batteries and the reassembled Linux battery (same layout).

Usage: parity_gate_closed.py <battery-out-dir> [--print]
"""
from __future__ import annotations

import json
import statistics
import sys
from pathlib import Path

FLOOR2_SHAPES = [
    "ycsb_a", "ycsb_b", "ycsb_c", "ycsb_d", "ycsb_e", "ycsb_f",
    "deps_cache_overwrite", "deps_lock_prewrite", "deps_mvcc_latest",
    "kvrocks_get", "kvrocks_set", "kvrocks_scan", "kvrocks_pipelined_set",
]
APPLY_3OF3 = "deps_apply_batch"
RAFTLOG_MIN = "deps_raftlog"
OPEN_SHAPES = ["deps_scan", "kvrocks_blob_set", "kvrocks_set_mc50"]

PAIRS = {  # shape name -> (compat leg, peer leg) within each round dir
    **{s: ("async", "rocks") for s in FLOOR2_SHAPES + [APPLY_3OF3, RAFTLOG_MIN]},
    "kvrocks_get": ("kvr", "rocks-kvr"),
    "kvrocks_set": ("kvr", "rocks-kvr"),
    "kvrocks_scan": ("kvr", "rocks-kvr"),
    "kvrocks_pipelined_set": ("kvr", "rocks-kvr"),
}


def qps(path: Path) -> dict[str, float]:
    try:
        d = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return {}
    return {b["name"]: float(b["qps"]) for b in d.get("benches", []) if b.get("qps")}


def round_ratios(out: Path, rnd: str) -> dict[str, float]:
    ratios: dict[str, float] = {}
    cache = {leg: qps(out / rnd / leg / "rocks_parity_bench.json")
             for leg in ("async", "kvr", "rocks", "rocks-kvr")}
    for shape, (cl, pl) in PAIRS.items():
        c, p = cache[cl].get(shape), cache[pl].get(shape)
        if c and p and p > 0:
            ratios[shape] = c / p
    return ratios


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    out = Path(sys.argv[1])
    rounds = sorted(d.name for d in out.glob("r[0-9]*")
                    if (d / "async").is_dir() or (d / "kvr").is_dir())
    if not rounds:
        print(f"parity-gate: no r*/ rounds under {out}", file=sys.stderr)
        return 2
    per_round = {r: round_ratios(out, r) for r in rounds}
    failures: list[str] = []
    print(f"parity-gate: {out} rounds={rounds} peer=RocksDB-default(sync=false)")

    for shape in FLOOR2_SHAPES:
        vals = [per_round[r][shape] for r in rounds if shape in per_round[r]]
        if not vals:
            failures.append(f"{shape}: no data")
            continue
        med = statistics.median(vals)
        ok = med >= 2.0
        print(f"  {'PASS' if ok else 'FAIL'} {shape:26s} median={med:6.3f}  rounds={['%.3f' % v for v in vals]}")
        if not ok:
            failures.append(f"{shape}: median {med:.3f} < 2.0")

    vals = [per_round[r].get(APPLY_3OF3) for r in rounds]
    if None in vals or not vals:
        failures.append(f"{APPLY_3OF3}: missing round data")
    else:
        ok = all(v >= 2.0 for v in vals)
        print(f"  {'PASS' if ok else 'FAIL'} {APPLY_3OF3:26s} 3/3={['%.3f' % v for v in vals]}")
        if not ok:
            failures.append(f"{APPLY_3OF3}: below 2.0 in a round")

    vals = [per_round[r].get(RAFTLOG_MIN) for r in rounds]
    if None in vals or not vals:
        failures.append(f"{RAFTLOG_MIN}: missing round data")
    else:
        med = statistics.median(vals)
        ok = med > 1.0
        print(f"  {'PASS' if ok else 'FAIL'} {RAFTLOG_MIN:26s} median={med:.3f} (target >1) rounds={['%.3f' % v for v in vals]}")
        if not ok:
            failures.append(f"{RAFTLOG_MIN}: median {med:.3f} <= 1.0")

    for shape in OPEN_SHAPES:
        vals = [per_round[r][shape] for r in rounds if shape in per_round[r]]
        if vals:
            print(f"  OPEN {shape:26s} (not gated): {['%.3f' % v for v in vals]}")

    if failures:
        print("parity-gate: REGRESSION — " + "; ".join(failures), file=sys.stderr)
        return 1
    print("parity-gate: PASS (closed shapes hold their floors)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
