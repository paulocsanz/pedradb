#!/usr/bin/env python3
"""RFC-0203 P0.2 — inventory-terminal gate (blocking, self-verifying).

The "Escada de contagem" inventory table in `docs/verification-ledger.md`
is the living map of work-count quotas (RFC-0199). This gate pins its
terminality:

- every inventory row ends in `count` (with the named catalog pair
  carrying a REGISTERED count row in scripts/ratchet/close_proofs.tsv)
  or `deferido` with a date AND a reason — any `todo` (or empty/unknown
  status) is RED: an open slice may live in an RFC, never in the ledger;
- every row names its `catalog:<id>` pair — an anonymous row cannot be
  checked;
- every registered count pair has its inventory row (a theorem landed
  without moving the ledger in the SAME commit is RED).

`--selftest` proves redness in memory: a planted `todo` row, a registered
pair that lost its row, a `deferido` without a date, a `count` row for an
unregistered pair, an anonymous row, and a new pair whose row did not
land in the SAME commit must each be caught; the healthy state must pass.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LEDGER = REPO / "docs" / "verification-ledger.md"
REGISTRY = REPO / "scripts" / "ratchet" / "close_proofs.tsv"

SECTION = "## Escada de contagem"
CATALOG_ID_RE = re.compile(r"catalog:[a-z0-9_]+")
DATE_RE = re.compile(r"\d{4}-\d{2}-\d{2}")


def parse_registry_counts(text: str) -> set[str]:
    out: set[str] = set()
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) == 5 and parts[0] == "count":
            out.add(parts[1])
    return out


def parse_inventory(ledger_text: str) -> list[list[str]]:
    """Data rows of the inventory table (4 cells each), in section order."""
    lines = ledger_text.splitlines()
    start = end = None
    for i, line in enumerate(lines):
        if start is None:
            if line.startswith(SECTION):
                start = i
        elif line.startswith("## "):
            end = i
            break
    if start is None:
        raise SystemExit(f"GATE inventory-terminal: FAIL — section '{SECTION}' not found in {LEDGER}")
    rows: list[list[str]] = []
    for line in lines[start:end if end is not None else len(lines)]:
        stripped = line.strip()
        if not stripped.startswith("|"):
            continue
        cells = [c.strip().replace("\\|", "|") for c in re.split(r"(?<!\\)\|", stripped.strip("|"))]
        if len(cells) != 4:
            raise SystemExit(f"GATE inventory-terminal: FAIL — inventory row needs 4 cells (Kernel|Medida|Cota|Status): {stripped!r}")
        if set(cells[0]) <= {"-", " "} and set(cells[1]) <= {"-", " "}:  # header separator
            continue
        if cells[0] == "Kernel":
            continue
        rows.append(cells)
    if not rows:
        raise SystemExit(f"GATE inventory-terminal: FAIL — inventory table in '{SECTION}' has no data rows")
    return rows


def check(rows: list[list[str]], registered: set[str]) -> list[str]:
    errs: list[str] = []
    seen: dict[str, int] = {}
    for cells in rows:
        kernel, _medida, _cota, status = cells
        ids = CATALOG_ID_RE.findall(kernel)
        if not ids:
            errs.append(f"row {kernel!r}: no `catalog:<id>` — an anonymous inventory row cannot be checked")
            continue
        cid = ids[0]
        if cid in seen:
            errs.append(f"{cid}: duplicate inventory row (one line per pair)")
        seen[cid] = seen.get(cid, 0) + 1
        if status.startswith("**count**"):
            if cid not in registered:
                errs.append(f"{cid}: inventory says count but no `count` row is registered in close_proofs.tsv — theorem missing")
        elif status.startswith("**deferido**"):
            rest = status[len("**deferido**"):].strip()
            if not DATE_RE.search(rest):
                errs.append(f"{cid}: deferido without a date (YYYY-MM-DD + reason required)")
            elif re.sub(r"[^a-zà-ú]", "", rest.lower(), flags=re.IGNORECASE) == "":
                errs.append(f"{cid}: deferido without a reason")
        else:
            head = status.split("(")[0].strip() or "(empty)"
            errs.append(
                f"{cid}: status {head!r} is not terminal — `count` (theorem registered) or "
                "`deferido` (dated reason); `todo` lives in the RFC, never in the ledger"
            )
    for cid in sorted(registered - set(seen)):
        errs.append(f"{cid}: registered count theorem has no inventory row — move the ledger in the SAME commit as the proof")
    return errs


def selftest() -> int:
    rows = parse_inventory(LEDGER.read_text(encoding="utf-8"))
    registered = parse_registry_counts(REGISTRY.read_text(encoding="utf-8"))
    base = check(rows, registered)
    if base:
        print("SELFTEST inventory-terminal: state already inconsistent — fix first:")
        for e in base:
            print(f"  {e}")
        return 1

    caught = 0
    total = 6

    # S1: planted `todo` row (an open slice leaking into the ledger).
    todo = [list(c) for c in rows] + [["Kernel novo (`catalog:new_pair`)", "medida", "cota", "**todo**"]]
    if check(todo, registered):
        print("SELFTEST inventory-terminal: caught=todo-row")
        caught += 1
    else:
        print("SELFTEST inventory-terminal: MISSED todo row")

    # S2: registered count pair that lost its inventory row.
    dropped = [c for c in rows if "catalog:bloom_may_contain" not in c[0]]
    if check(dropped, registered):
        print("SELFTEST inventory-terminal: caught=theorem-without-row")
        caught += 1
    else:
        print("SELFTEST inventory-terminal: MISSED theorem without row")

    # S3: deferido without a date.
    undated = [list(c) for c in rows]
    undated[0][3] = "**deferido** (razão sem data)"
    if check(undated, registered):
        print("SELFTEST inventory-terminal: caught=deferido-undated")
        caught += 1
    else:
        print("SELFTEST inventory-terminal: MISSED deferido without date")

    # S4: count row for a pair that never registered a theorem.
    phantom = [list(c) for c in rows] + [
        ["Kernel novo (`catalog:new_pair`)", "medida", "cota", "**count** (phantom)"]
    ]
    if check(phantom, registered):
        print("SELFTEST inventory-terminal: caught=count-unregistered")
        caught += 1
    else:
        print("SELFTEST inventory-terminal: MISSED unregistered count")

    # S5: anonymous row (no catalog id — cannot be checked).
    anon = [list(c) for c in rows] + [["Kernel anônimo", "medida", "cota", "**count**"]]
    if check(anon, registered):
        print("SELFTEST inventory-terminal: caught=anonymous-row")
        caught += 1
    else:
        print("SELFTEST inventory-terminal: MISSED anonymous row")

    # S6 (RFC-0203 P2.1): new pair whose count theorem landed but whose
    # inventory row did not land in the SAME commit.
    if check([list(c) for c in rows], registered | {"catalog:new_pair"}):
        print("SELFTEST inventory-terminal: caught=new-pair-without-row")
        caught += 1
    else:
        print("SELFTEST inventory-terminal: MISSED new pair without row")

    print(f"SELFTEST inventory-terminal: {caught}/{total} sabotages caught")
    return 0 if caught == total else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--selftest", action="store_true", help="prove all red directions")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    rows = parse_inventory(LEDGER.read_text(encoding="utf-8"))
    registered = parse_registry_counts(REGISTRY.read_text(encoding="utf-8"))
    errs = check(rows, registered)
    if errs:
        for e in errs:
            print(f"GATE inventory-terminal: FAIL — {e}")
        print("GATE inventory-terminal: RED")
        return 1
    n_count = sum(1 for c in rows if c[3].startswith("**count**"))
    n_def = sum(1 for c in rows if c[3].startswith("**deferido**"))
    print(
        f"GATE inventory-terminal: GREEN — {len(rows)} inventory rows terminal: "
        f"{n_count} count (all registered), {n_def} deferido (dated), 0 todo"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
