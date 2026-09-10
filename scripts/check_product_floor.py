#!/usr/bin/env python3
"""RFC-0191 P0.1 — product-guarantee ratchet (blocking, self-verifying).

Four product rows (D1/R1/T1/C1) live in
``scripts/ratchet/product_guarantees.tsv``. Layer only moves UP
(model → atom → close). ``floor_promoted N`` is the minimum number of
atom|close rows — dropping R1 from atom back to model at the day-1
freeze (N=1) is red.

atom/close rows reuse the RFC-0188 credit rule: theorem exists, statement
carries a forall-binder (named property over all inputs, not ``rfl`` on
concrete inputs), Lean file has zero ``sorry``, catalog id resolves.

``--selftest`` proves redness in memory: layer descent, theorem without
forall, dangling catalog id must each be caught; the honest freeze must
pass (an always-red checker is also a bug).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TSV = REPO / "scripts" / "ratchet" / "product_guarantees.tsv"
CATALOG = REPO / "scripts" / "formal" / "catalog.json"

REQUIRED_IDS = ("D1", "R1", "T1", "C1")
LAYERS = ("model", "atom", "close")


def parse_tsv(text: str) -> tuple[int, list[dict[str, str]]]:
    floor = None
    rows: list[dict[str, str]] = []
    for no, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if line.startswith("floor_promoted "):
            floor = int(line.split()[1])
            continue
        parts = line.split()
        if len(parts) != 6:
            raise SystemExit(f"GATE product-floor: FAIL — line {no} needs 6 fields: {raw!r}")
        pid, layer, catalog_id, theorem, lean_file, entry = parts
        if pid not in REQUIRED_IDS:
            raise SystemExit(f"GATE product-floor: FAIL — unknown id {pid!r} (want D1|R1|T1|C1)")
        if layer not in LAYERS:
            raise SystemExit(f"GATE product-floor: FAIL — {pid} layer must be model|atom|close, got {layer!r}")
        rows.append(
            {
                "id": pid,
                "layer": layer,
                "catalog_id": catalog_id,
                "theorem": theorem,
                "lean_file": lean_file,
                "entry": entry,
            }
        )
    if floor is None:
        raise SystemExit("GATE product-floor: FAIL — missing `floor_promoted N`")
    seen = [r["id"] for r in rows]
    if sorted(seen) != sorted(REQUIRED_IDS):
        raise SystemExit(f"GATE product-floor: FAIL — need exactly {REQUIRED_IDS}, got {seen}")
    return floor, rows


def theorem_errors(row: dict[str, str], catalog_ids: set[str]) -> list[str]:
    """Credit rule for atom/close. model rows skip (no close credit)."""
    tag = f"{row['id']} {row['layer']}"
    errs: list[str] = []
    cid = row["catalog_id"]
    if not cid.startswith("catalog:"):
        return [f"{tag}: catalog_id must be `catalog:<id>`, got {cid!r}"]
    cid = cid[len("catalog:") :]
    if cid not in catalog_ids:
        errs.append(f"{tag}: catalog:{cid} does not exist — dangling product claim")
    if row["layer"] == "model":
        return errs
    if row["theorem"] == "-" or row["lean_file"] == "-":
        errs.append(f"{tag}: atom/close row needs a theorem and lean_file, not `-`")
        return errs
    path = REPO / row["lean_file"]
    if not path.is_file():
        errs.append(f"{tag}: lean file {row['lean_file']} does not exist")
        return errs
    text = path.read_text(encoding="utf-8")
    if "sorry" in text:
        errs.append(f"{tag}: {row['lean_file']} contains `sorry` — no product credit on a sorry file")
    m = re.search(rf"\btheorem\s+{re.escape(row['theorem'])}\b(.*?):=", text, re.DOTALL)
    if not m:
        errs.append(f"{tag}: no `theorem {row['theorem']}` in {row['lean_file']}")
        return errs
    stmt = m.group(1)
    if "∀" not in stmt and "\\forall" not in stmt:
        errs.append(
            f"{tag}: statement has no forall-binder — product atom/close requires a named "
            "property over all inputs, not a definitional equality over concrete inputs"
        )
    return errs


def check(floor: int, rows: list[dict[str, str]], catalog_ids: set[str]) -> list[str]:
    errs: list[str] = []
    promoted = sum(1 for r in rows if r["layer"] in ("atom", "close"))
    if promoted < floor:
        errs.append(
            f"promoted {promoted} < floor_promoted {floor} — a product row descended "
            "to model (layer only moves UP; freeze day-1 needs ≥1 atom|close)"
        )
    for row in rows:
        errs.extend(theorem_errors(row, catalog_ids))
    return errs


def selftest() -> int:
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    catalog_ids = {p["id"] for p in catalog["pairs"]}
    floor, rows = parse_tsv(TSV.read_text(encoding="utf-8"))
    base = check(floor, rows, catalog_ids)
    if base:
        print("SELFTEST product-floor: state already inconsistent — fix first:")
        for e in base:
            print(f"  {e}")
        return 1

    caught = 0
    total = 4

    # S1: layer descent (R1 atom → model) drops promoted below floor.
    descended = [dict(r) for r in rows]
    for r in descended:
        if r["id"] == "R1":
            r["layer"] = "model"
            r["theorem"] = "-"
            r["lean_file"] = "-"
    if check(floor, descended, catalog_ids):
        print("SELFTEST product-floor: caught=layer-descent")
        caught += 1
    else:
        print("SELFTEST product-floor: MISSED layer-descent")

    # S2: atom/close theorem without a forall-binder.
    no_forall = [dict(r) for r in rows]
    for r in no_forall:
        if r["id"] == "R1":
            r["theorem"] = "visible_at_deletion"  # concrete rfl, no ∀
    if check(floor, no_forall, catalog_ids):
        print("SELFTEST product-floor: caught=no-forall")
        caught += 1
    else:
        print("SELFTEST product-floor: MISSED no-forall")

    # S3: dangling catalog id.
    dangling = [dict(r) for r in rows]
    for r in dangling:
        if r["id"] == "D1":
            r["catalog_id"] = "catalog:no_such_pair"
    if check(floor, dangling, catalog_ids):
        print("SELFTEST product-floor: caught=dangling-catalog")
        caught += 1
    else:
        print("SELFTEST product-floor: MISSED dangling-catalog")

    # S4: honest freeze must PASS (checker not always-red).
    if check(floor, rows, catalog_ids) == []:
        print("SELFTEST product-floor: passed=honest-freeze (oracle not always-red)")
        caught += 1
    else:
        print("SELFTEST product-floor: honest freeze REJECTED — oracle always-red")

    print(f"SELFTEST product-floor: {caught}/{total} checks caught")
    return 0 if caught == total else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--selftest", action="store_true", help="prove all red directions")
    args = ap.parse_args()
    if args.selftest:
        return selftest()

    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    catalog_ids = {p["id"] for p in catalog["pairs"]}
    floor, rows = parse_tsv(TSV.read_text(encoding="utf-8"))
    errs = check(floor, rows, catalog_ids)
    if errs:
        for e in errs:
            print(f"GATE product-floor: FAIL — {e}")
        print("GATE product-floor: RED")
        return 1
    layers = " ".join(f"{r['id']}={r['layer']}" for r in rows)
    promoted = sum(1 for r in rows if r["layer"] in ("atom", "close"))
    print(
        f"GATE product-floor: GREEN — {layers}, promoted={promoted}>=floor {floor}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
