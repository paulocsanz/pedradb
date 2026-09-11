#!/usr/bin/env python3
"""RFC-0188 P0.1/P0.3 — formal depth ratchet (blocking, self-verifying).

The proof ladder is extract -> close -> atom (RFC-0188). This gate pins it:

- floors (`scripts/ratchet/proof_depth.tsv`): the ladder counts REGISTERED
  proofs — a registered kind below its floor = red (proof regressed);
  floors only move up, same commit as the proof;
- cap: `glue.data_fate` ABOVE cap = red — a new data-fate atom without an
  equivalent removal (trampoline monotonicity);
- registry (`scripts/ratchet/close_proofs.tsv`): every close/atom/count row must
  resolve — theorem exists in the Lean file, statement carries a
  forall-binder (named property, not definitional equality over concrete
  inputs), file has zero `sorry`, catalog id resolves in catalog.json;
- `floor_count` (RFC-0199): count rows are the work-count credit (one per
  catalog pair; orthogonal to close/atom — a pair may carry both). Floor
  counts registered count rows and only moves up;
- `residuals.json` `proof_depth.close/.atom/.count` must EQUAL the LIVE count
  (registered proofs + unregistered close/atom twins without extraction,
  same rule as `pedra_formal.py`; count = registered rows) — stale residual
  = red: counts move with the proof, never after.

`--selftest` proves redness in memory: raised floor, stale residual,
grown data_fate, a broken registration row, and a stale count residual
must each be caught; the healthy state must pass.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
FLOORS = REPO / "scripts" / "ratchet" / "proof_depth.tsv"
REGISTRY = REPO / "scripts" / "ratchet" / "close_proofs.tsv"
RESIDUALS = REPO / "scripts" / "formal" / "residuals.json"
CATALOG = REPO / "scripts" / "formal" / "catalog.json"

INT_KEYS = ("floor_extract", "floor_close", "floor_atom", "floor_count", "cap_data_fate", "handler_loc")


def parse_floors(text: str) -> dict[str, int]:
    out: dict[str, int] = {}
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        k, _, v = line.partition(" ")
        if k not in INT_KEYS:
            raise SystemExit(f"GATE depth-floor: FAIL — unknown key {k!r} in proof_depth.tsv")
        out[k] = int(v.strip())
    missing = [k for k in INT_KEYS if k not in out]
    if missing:
        raise SystemExit(f"GATE depth-floor: FAIL — proof_depth.tsv missing {missing}")
    return out


def parse_registry(text: str) -> list[dict[str, str]]:
    rows = []
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) != 5:
            raise SystemExit(f"GATE depth-floor: FAIL — close_proofs.tsv row needs 5 fields: {line!r}")
        kind, catalog_id, theorem, lean_file, entry = parts
        if kind not in ("close", "atom", "count"):
            raise SystemExit(f"GATE depth-floor: FAIL — registry kind must be close|atom|count: {line!r}")
        rows.append(
            {"kind": kind, "catalog_id": catalog_id, "theorem": theorem, "lean_file": lean_file, "entry": entry}
        )
    seen: set[tuple[str, str]] = set()
    for row in rows:
        key = (row["catalog_id"], row["kind"])
        if key in seen:
            raise SystemExit(
                f"GATE depth-floor: FAIL — duplicate registry row {row['kind']} {row['catalog_id']} "
                "(one credit per pair per kind)"
            )
        seen.add(key)
    return rows


def load_actual() -> dict[str, int]:
    r = json.loads(RESIDUALS.read_text(encoding="utf-8"))
    depth = r["glue"]["proof_depth"]
    return {
        "extract": depth["extract"],
        "close": depth["close"],
        "atom": depth["atom"],
        "count": depth["count"],
        "data_fate": r["glue"]["data_fate"],
        "handler_loc": r["glue"]["handler_loc"],
    }


def registry_errors(rows: list[dict[str, str]], catalog_ids: set[str]) -> list[str]:
    errs: list[str] = []
    for row in rows:
        tag = f"{row['kind']} {row['theorem']}"
        cid = row["catalog_id"]
        if cid.startswith("catalog:"):
            cid = cid[len("catalog:") :]
        else:
            errs.append(f"{tag}: catalog_id must be `catalog:<id>` form, got {row['catalog_id']!r}")
        if cid not in catalog_ids:
            errs.append(f"{tag}: catalog:{cid} does not exist — theorem about a non-production pair")
        path = REPO / row["lean_file"]
        if not path.exists():
            errs.append(f"{tag}: lean file {row['lean_file']} does not exist")
            continue
        text = path.read_text(encoding="utf-8")
        if "sorry" in text:
            errs.append(f"{tag}: {row['lean_file']} contains `sorry` — no proof credit on a sorry file")
        m = re.search(rf"\btheorem\s+{re.escape(row['theorem'])}\b(.*?):=", text, re.DOTALL)
        if not m:
            errs.append(f"{tag}: no `theorem {row['theorem']}` in {row['lean_file']}")
            continue
        stmt = m.group(1)
        if "∀" not in stmt and "\\forall" not in stmt:
            errs.append(
                f"{tag}: statement has no forall-binder — close/atom credit requires a named "
                "property over all inputs, not a definitional equality over concrete inputs"
            )
    return errs


def registered_map() -> dict[str, str]:
    """Ladder depth overrides: close/atom rows only. A `count` row is the
    RFC-0199 work credit — orthogonal to the correctness ladder, so it never
    overrides a pair's depth (a pair can carry close AND count)."""
    out: dict[str, str] = {}
    for row in parse_registry(REGISTRY.read_text(encoding="utf-8")):
        if row["kind"] == "count":
            continue
        cid = row["catalog_id"]
        if cid.startswith("catalog:"):
            cid = cid[len("catalog:") :]
        out[cid] = row["kind"]
    return out


def registered_count_ids() -> set[str]:
    """Catalog ids carrying a registered `count` row (RFC-0199 work credit)."""
    out: set[str] = set()
    for row in parse_registry(REGISTRY.read_text(encoding="utf-8")):
        if row["kind"] != "count":
            continue
        cid = row["catalog_id"]
        if cid.startswith("catalog:"):
            cid = cid[len("catalog:") :]
        out.add(cid)
    return out


def live_counts(catalog: dict, registered: dict[str, str]) -> dict[str, int]:
    """Same rule as pedra_formal.py proof_depth: a REGISTERED pair counts
    at its registered ladder step (close = forall theorem over the
    extracted body); an UNREGISTERED twin counts at its twin_kind step
    only when its kernel has no Aeneas extract (RFC-0170 twin semantics).
    The kernel-extraction authority (AENEAS_EXTRACTS) is imported from
    pedra_formal so the two gates agree by construction.
    RFC-0199 `count`: every registered count row counts once (the work
    credit is orthogonal to the ladder); a MODEL-tier pair with a count
    row graduates out of the model stand-in pool into the count tier."""
    sys.path.insert(0, str(REPO / "scripts" / "formal"))
    import pedra_formal

    count_ids = registered_count_ids()
    close = atom = count = 0
    for pair in catalog["pairs"]:
        pid = pair["id"]
        reg = registered.get(pid)
        if reg == "close":
            close += 1
        elif reg == "atom":
            atom += 1
        elif pair.get("twin_kind") == "atom":
            atom += 1
        elif pair.get("twin_kind") == "close":
            kernel = pair.get("kernel") or ""
            if not any(kernel == k for k, _ in pedra_formal.AENEAS_EXTRACTS):
                close += 1
        # model + count row → graduated (no longer a stand-in)
    count = sum(1 for row in parse_registry(REGISTRY.read_text(encoding="utf-8")) if row["kind"] == "count")
    return {"close": close, "atom": atom, "count": count}


def check(
    floors: dict[str, int],
    actual: dict[str, int],
    rows: list[dict[str, str]],
    reg_errs: list[str],
    live: dict[str, int],
) -> list[str]:
    errs: list[str] = []
    if actual["extract"] < floors["floor_extract"]:
        errs.append(
            f"extract depth {actual['extract']} below floor {floors['floor_extract']} — "
            "an extraction regressed; regain it or move the floor in the SAME commit"
        )
    reg_close = sum(1 for r in rows if r["kind"] == "close")
    reg_atom = sum(1 for r in rows if r["kind"] == "atom")
    if reg_close < floors["floor_close"]:
        errs.append(
            f"registered close proofs {reg_close} below floor {floors['floor_close']} — "
            "a ladder proof regressed; floors only move up (RFC-0188)"
        )
    if reg_atom < floors["floor_atom"]:
        errs.append(
            f"registered atom proofs {reg_atom} below floor {floors['floor_atom']} — "
            "a ladder proof regressed; floors only move up (RFC-0188)"
        )
    reg_count = sum(1 for r in rows if r["kind"] == "count")
    if reg_count < floors["floor_count"]:
        errs.append(
            f"registered count proofs {reg_count} below floor {floors['floor_count']} — "
            "a work-count proof regressed; floors only move up (RFC-0199)"
        )
    for key in ("close", "atom", "count"):
        if actual[key] != live[key]:
            errs.append(
                f"residuals proof_depth.{key}={actual[key]} but live count is {live[key]} — "
                "stale residual: counts move in the SAME commit as the proof"
            )
    if actual["data_fate"] > floors["cap_data_fate"]:
        errs.append(
            f"data_fate {actual['data_fate']} above cap {floors['cap_data_fate']} — new data-fate "
            "atom without equivalent removal; empty the trampoline, never grow it"
        )
    errs.extend(reg_errs)
    return errs


def selftest() -> int:
    floors = parse_floors(FLOORS.read_text(encoding="utf-8"))
    rows = parse_registry(REGISTRY.read_text(encoding="utf-8"))
    actual = load_actual()
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    catalog_ids = {p["id"] for p in catalog["pairs"]}
    registered = registered_map()
    live = live_counts(catalog, registered)
    reg_errs = registry_errors(rows, catalog_ids)
    base = check(floors, actual, rows, reg_errs, live)
    if base:
        print("SELFTEST depth-floor: state already inconsistent — fix first:")
        for e in base:
            print(f"  {e}")
        return 1

    caught = 0
    total = 5

    # S1: floor raised beyond the registered ladder proofs.
    tampered_floors = dict(floors)
    tampered_floors["floor_close"] = live["close"] + 1
    if check(tampered_floors, actual, rows, reg_errs, live):
        print("SELFTEST depth-floor: caught=shrunk-floor")
        caught += 1
    else:
        print("SELFTEST depth-floor: MISSED shrunk floor")

    # S2: stale residual (live count moved, residuals not moved in the same commit).
    tampered_actual = dict(actual)
    tampered_actual["close"] = live["close"] + 1
    if check(floors, tampered_actual, rows, reg_errs, live):
        print("SELFTEST depth-floor: caught=stale-residual")
        caught += 1
    else:
        print("SELFTEST depth-floor: MISSED stale residual")

    # S3: trampoline grew a data-fate atom past the cap.
    tampered_actual = dict(actual)
    tampered_actual["data_fate"] = floors["cap_data_fate"] + 1
    if check(floors, tampered_actual, rows, reg_errs, live):
        print("SELFTEST depth-floor: caught=data-fate-growth")
        caught += 1
    else:
        print("SELFTEST depth-floor: MISSED data-fate growth")

    # S4: registration row that does not resolve (no such theorem anywhere).
    fake = [
        {
            "kind": "close",
            "catalog_id": "catalog:vote",
            "theorem": "no_such_theorem_anywhere",
            "lean_file": "formal/aeneas/lean/Ae.lean",
            "entry": "never",
        }
    ]
    if check(floors, actual, rows + fake, reg_errs + registry_errors(fake, catalog_ids), live):
        print("SELFTEST depth-floor: caught=broken-registration")
        caught += 1
    else:
        print("SELFTEST depth-floor: MISSED broken registration")

    # S5 (RFC-0199): stale count residual — a count row landed without
    # moving residuals.proof_depth.count in the same commit.
    tampered_actual = dict(actual)
    tampered_actual["count"] = live["count"] + 1
    if check(floors, tampered_actual, rows, reg_errs, live):
        print("SELFTEST depth-floor: caught=stale-count-residual")
        caught += 1
    else:
        print("SELFTEST depth-floor: MISSED stale count residual")

    print(f"SELFTEST depth-floor: {caught}/{total} sabotages caught")
    return 0 if caught == total else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--selftest", action="store_true", help="prove all red directions")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    floors = parse_floors(FLOORS.read_text(encoding="utf-8"))
    rows = parse_registry(REGISTRY.read_text(encoding="utf-8"))
    actual = load_actual()
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    catalog_ids = {p["id"] for p in catalog["pairs"]}
    registered = registered_map()
    live = live_counts(catalog, registered)
    errs = check(floors, actual, rows, registry_errors(rows, catalog_ids), live)
    if errs:
        for e in errs:
            print(f"GATE depth-floor: FAIL — {e}")
        print("GATE depth-floor: RED")
        return 1
    reg_close = sum(1 for r in rows if r["kind"] == "close")
    reg_atom = sum(1 for r in rows if r["kind"] == "atom")
    reg_count = sum(1 for r in rows if r["kind"] == "count")
    print(
        f"GATE depth-floor: GREEN — extract={actual['extract']} (floor {floors['floor_extract']}), "
        f"registered ladder close={reg_close} (floor {floors['floor_close']}) / "
        f"atom={reg_atom} (floor {floors['floor_atom']}), residuals close={actual['close']}"
        f"/atom={actual['atom']} == live {live['close']}/{live['atom']}, "
        f"count={reg_count} (floor {floors['floor_count']}, residual {actual['count']} == live {live['count']}), "
        f"data_fate={actual['data_fate']}<={floors['cap_data_fate']}, "
        f"handler_loc={actual['handler_loc']} (series; TSV {floors['handler_loc']})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
