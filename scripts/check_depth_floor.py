#!/usr/bin/env python3
"""RFC-0188 P0.1/P0.3 — formal depth ratchet (blocking, self-verifying).

The proof ladder is extract -> close -> atom (RFC-0188). This gate pins it:

- floors (`scripts/ratchet/proof_depth.tsv`): actual depth BELOW floor =
  red — a proof regressed; floors only move up, same commit as the proof;
- cap: `glue.data_fate` ABOVE cap = red — a new data-fate atom without an
  equivalent removal (trampoline monotonicity);
- registry (`scripts/ratchet/close_proofs.tsv`): every close/atom row must
  resolve — theorem exists in the Lean file, statement carries a
  forall-binder (named property, not definitional equality over concrete
  inputs), file has zero `sorry`, catalog id resolves in catalog.json;
- `residuals.json` `proof_depth.close/.atom` must EQUAL the registry
  counts (stale residual = red — counts move with the proof, never after).

`--selftest` proves redness in memory: raised floor, stale residual,
grown data_fate, and a broken registration row must each be caught; the
healthy state must pass.
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

INT_KEYS = ("floor_extract", "floor_close", "floor_atom", "cap_data_fate", "handler_loc")


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
        if kind not in ("close", "atom"):
            raise SystemExit(f"GATE depth-floor: FAIL — registry kind must be close|atom: {line!r}")
        rows.append(
            {"kind": kind, "catalog_id": catalog_id, "theorem": theorem, "lean_file": lean_file, "entry": entry}
        )
    return rows


def load_actual() -> dict[str, int]:
    r = json.loads(RESIDUALS.read_text(encoding="utf-8"))
    depth = r["glue"]["proof_depth"]
    return {
        "extract": depth["extract"],
        "close": depth["close"],
        "atom": depth["atom"],
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


def check(
    floors: dict[str, int],
    actual: dict[str, int],
    rows: list[dict[str, str]],
    reg_errs: list[str],
) -> list[str]:
    errs: list[str] = []
    for key, floor_key in (("extract", "floor_extract"), ("close", "floor_close"), ("atom", "floor_atom")):
        if actual[key] < floors[floor_key]:
            errs.append(
                f"{key} depth {actual[key]} below floor {floors[floor_key]} — a proof regressed; "
                "regain it or move the floor in the SAME commit as the proof that earned it"
            )
    reg_close = sum(1 for r in rows if r["kind"] == "close")
    reg_atom = sum(1 for r in rows if r["kind"] == "atom")
    for key, reg in (("close", reg_close), ("atom", reg_atom)):
        if actual[key] != reg:
            errs.append(
                f"residuals proof_depth.{key}={actual[key]} but registry has {reg} — "
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
    catalog_ids = {p["id"] for p in json.loads(CATALOG.read_text(encoding="utf-8"))["pairs"]}
    reg_errs = registry_errors(rows, catalog_ids)
    base = check(floors, actual, rows, reg_errs)
    if base:
        print("SELFTEST depth-floor: state already inconsistent — fix first:")
        for e in base:
            print(f"  {e}")
        return 1

    caught = 0
    total = 4

    # S1: floor raised beyond what is delivered (proof regressed / floor moved early).
    tampered_floors = dict(floors)
    tampered_floors["floor_close"] = actual["close"] + 1
    if check(tampered_floors, actual, rows, reg_errs):
        print("SELFTEST depth-floor: caught=shrunk-floor")
        caught += 1
    else:
        print("SELFTEST depth-floor: MISSED shrunk floor")

    # S2: stale residual (registry gained a proof, residuals not moved in the same commit).
    tampered_actual = dict(actual)
    tampered_actual["close"] = actual["close"] + 1
    if check(floors, tampered_actual, rows, reg_errs):
        print("SELFTEST depth-floor: caught=stale-residual")
        caught += 1
    else:
        print("SELFTEST depth-floor: MISSED stale residual")

    # S3: trampoline grew a data-fate atom past the cap.
    tampered_actual = dict(actual)
    tampered_actual["data_fate"] = floors["cap_data_fate"] + 1
    if check(floors, tampered_actual, rows, reg_errs):
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
    if check(floors, actual, rows + fake, reg_errs + registry_errors(fake, catalog_ids)):
        print("SELFTEST depth-floor: caught=broken-registration")
        caught += 1
    else:
        print("SELFTEST depth-floor: MISSED broken registration")

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
    catalog_ids = {p["id"] for p in json.loads(CATALOG.read_text(encoding="utf-8"))["pairs"]}
    errs = check(floors, actual, rows, registry_errors(rows, catalog_ids))
    if errs:
        for e in errs:
            print(f"GATE depth-floor: FAIL — {e}")
        print("GATE depth-floor: RED")
        return 1
    reg_close = sum(1 for r in rows if r["kind"] == "close")
    reg_atom = sum(1 for r in rows if r["kind"] == "atom")
    print(
        f"GATE depth-floor: GREEN — extract={actual['extract']} (floor {floors['floor_extract']}), "
        f"close={actual['close']} (floor {floors['floor_close']}), atom={actual['atom']} "
        f"(floor {floors['floor_atom']}), data_fate={actual['data_fate']}<={floors['cap_data_fate']}, "
        f"registry {reg_close} close/{reg_atom} atom rows resolve, "
        f"handler_loc={actual['handler_loc']} (series; TSV {floors['handler_loc']})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
