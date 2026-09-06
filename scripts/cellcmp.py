#!/usr/bin/env python3
"""Compare two snapshot-bench cell artifacts (RFC-0168 P0.3).

Each artifact is the CSV written by `cellcost::flush_group` (env
`SLIPSTREAM_BENCH_ARTIFACT`), one row per criterion cell:
`run,cell,backend,median_ns,...cost columns`.

The comparator flags any cell whose median_ns regressed more than
--threshold% (default 3%) from artifact A (baseline) to artifact B (new).
Exit code 1 when at least one regression is flagged, 0 otherwise. Cells
present in only one artifact, or with a non-numeric median (NA), are
reported as warnings and never count as regressions.

Usage: cellcmp.py BASELINE.csv NEW.csv [--threshold 3.0]
"""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path


def load(path: Path) -> dict[tuple[str, str], dict[str, str]]:
    rows: dict[tuple[str, str], dict[str, str]] = {}
    with path.open(newline="") as fh:
        for row in csv.DictReader(fh):
            key = (row["cell"], row["backend"])
            if key in rows:
                raise SystemExit(f"{path}: duplicate cell {key}")
            rows[key] = row
    return rows


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("baseline", type=Path)
    ap.add_argument("new", type=Path)
    ap.add_argument("--threshold", type=float, default=3.0,
                    help="median regression %% that flags a cell (default 3.0)")
    args = ap.parse_args()

    base = load(args.baseline)
    new = load(args.new)

    only_base = sorted(set(base) - set(new))
    only_new = sorted(set(new) - set(base))
    for key in only_base:
        print(f"WARN only in baseline: {key[0]}/{key[1]}")
    for key in only_new:
        print(f"WARN only in new:      {key[0]}/{key[1]}")

    flagged = 0
    compared = 0
    for key in sorted(set(base) & set(new)):
        a, b = base[key].get("median_ns", "NA"), new[key].get("median_ns", "NA")
        if a == "NA" or b == "NA":
            print(f"WARN median NA, skipped: {key[0]}/{key[1]} ({a} -> {b})")
            continue
        try:
            fa, fb = float(a), float(b)
        except ValueError:
            print(f"WARN median unparsable, skipped: {key[0]}/{key[1]}")
            continue
        if fa <= 0:
            continue
        compared += 1
        pct = (fb - fa) / fa * 100.0
        mark = ""
        if pct > args.threshold:
            mark = "  <-- REGRESSION"
            flagged += 1
        print(f"{key[0]}/{key[1]:<22} {fa:>14.1f} -> {fb:>14.1f} ns  {pct:+7.2f}%{mark}")

    print(f"\ncompared {compared} cells; threshold {args.threshold:+.1f}%; "
          f"regressions flagged: {flagged}")
    return 1 if flagged else 0


if __name__ == "__main__":
    sys.exit(main())
