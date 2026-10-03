#!/usr/bin/env python3
"""RFC-0187 P1.2 — ledger consistency gate (blocking, self-verifying).

`docs/verification-ledger.md` is the authoritative theorem/experiment/TCB
layer ledger. Its machine-checked tier is cross-checked against
`scripts/formal/catalog.json`:

- the ``<!-- ledger-catalog: ... -->`` count marker must EQUAL the counts
  computed live from the catalog (stale ledger after a catalog change =
  red — the ledger must move in the same commit);
- every ``catalog:<id>`` pointer in the ledger must resolve to a real
  catalog pair id (dangling theorem claim = red).

``--selftest`` proves redness in memory: a tampered count and a dangling
pointer must each be caught; the healthy ledger must pass.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LEDGER = REPO / "docs" / "verification-ledger.md"
CATALOG = REPO / "scripts" / "formal" / "catalog.json"

MARKER = re.compile(r"<!--\s*ledger-catalog:\s*(.*?)\s*-->")
POINTER = re.compile(r"`catalog:([A-Za-z0-9_]+)`")


def catalog_counts(c: dict) -> dict[str, int]:
    pairs = c["pairs"]
    return {
        "total": len(pairs),
        "proof": sum(1 for p in pairs if not p["id"].startswith("l28_")),
        "campaign": sum(1 for p in pairs if p["id"].startswith("l28_")),
        "absent": sum(1 for p in pairs if p.get("status") == "absent"),
        "single_artifact": sum(1 for p in pairs if p.get("single_artifact")),
        "aeneas_scripts": sum(1 for p in pairs if p.get("aeneas")),
        "clones": len(c["clones"]),
        "models": len(c["models"]),
    }


def parse_marker(text: str) -> dict[str, int]:
    m = MARKER.search(text)
    if not m:
        raise SystemExit("GATE ledger: FAIL — no <!-- ledger-catalog: ... --> marker in ledger")
    out: dict[str, int] = {}
    for part in m.group(1).split():
        k, _, v = part.partition("=")
        out[k] = int(v)
    return out


def check(marker_counts: dict[str, int], live: dict[str, int], pointers: list[str], ids: set[str]) -> list[str]:
    errs = []
    for k in sorted(set(marker_counts) | set(live)):
        m, l = marker_counts.get(k), live.get(k)
        if m != l:
            errs.append(
                f"ledger marker {k}={m} but catalog says {l} — move the ledger in the SAME commit as the catalog change"
            )
    for p in pointers:
        if p not in ids:
            errs.append(f"ledger points at catalog:{p} which does not exist — dangling theorem claim")
    return errs


def selftest() -> int:
    ledger = LEDGER.read_text(encoding="utf-8")
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    live = catalog_counts(catalog)
    ids = {p["id"] for p in catalog["pairs"]}
    marker = parse_marker(ledger)
    pointers = POINTER.findall(ledger)
    base = check(marker, live, pointers, ids)
    if base:
        print("SELFTEST ledger: ledger already inconsistent — fix first:")
        for e in base:
            print(f"  {e}")
        return 1

    caught = 0
    total = 2

    # S1: tampered count (stale ledger after a catalog change).
    tampered = dict(marker)
    tampered["proof"] += 1
    if check(tampered, live, pointers, ids):
        print("SELFTEST ledger: caught=stale-count")
        caught += 1
    else:
        print("SELFTEST ledger: MISSED stale count")

    # S2: dangling catalog pointer.
    if check(marker, live, pointers + ["vote_of_no_confidence"], ids):
        print("SELFTEST ledger: caught=dangling-pointer")
        caught += 1
    else:
        print("SELFTEST ledger: MISSED dangling pointer")

    print(f"SELFTEST ledger: {caught}/{total} sabotages caught")
    return 0 if caught == total else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--selftest", action="store_true", help="prove both red directions")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    ledger = LEDGER.read_text(encoding="utf-8")
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    live = catalog_counts(catalog)
    ids = {p["id"] for p in catalog["pairs"]}
    marker = parse_marker(ledger)
    pointers = POINTER.findall(ledger)
    errs = check(marker, live, pointers, ids)
    if errs:
        for e in errs:
            print(f"GATE ledger: FAIL — {e}")
        print("GATE ledger: RED")
        return 1
    print(
        f"GATE ledger: GREEN — {len(pointers)} catalog pointers resolve, "
        f"counts match (total={live['total']} proof={live['proof']} campaign={live['campaign']})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
