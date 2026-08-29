#!/usr/bin/env python3
"""RFC-0151: a data_fate pair without dst_plant must FAIL naming that id."""

from __future__ import annotations

import copy
import io
import json
import sys
from contextlib import redirect_stdout
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import pedra_formal as pf  # noqa: E402


def main() -> int:
    catalog = json.loads((ROOT / "scripts/formal/catalog.json").read_text(encoding="utf-8"))
    live = pf.Report()
    pf.check_three_teeth(ROOT, catalog, live)
    if live.failed:
        print("FAIL live three teeth:", live.failed)
        return 1
    print("ok live three teeth freeze")

    fate = [p for p in catalog["pairs"] if p.get("data_fate")]
    if not fate:
        print("FAIL: no data_fate pairs")
        return 1
    dropped = fate[0]["id"]
    mutant_cat = copy.deepcopy(catalog)
    for p in mutant_cat["pairs"]:
        if p.get("id") == dropped:
            p.pop("dst_plant", None)
            break
    mutant = pf.Report()
    with redirect_stdout(io.StringIO()):
        pf.check_three_teeth(ROOT, mutant_cat, mutant)
    if not any(dropped in m and "dst_plant" in m for m in mutant.failed):
        print("FAIL drop did not name", dropped, "got", mutant.failed)
        return 1
    print(f"ok dst_plant-drop names {dropped}")

    if len(fate) < 2:
        print("FAIL: need two data_fate pairs for as_is drop")
        return 1
    dropped_as = fate[1]["id"]
    mutant_cat = copy.deepcopy(catalog)
    for p in mutant_cat["pairs"]:
        if p.get("id") == dropped_as:
            p.pop("as_is", None)
            break
    mutant = pf.Report()
    with redirect_stdout(io.StringIO()):
        pf.check_three_teeth(ROOT, mutant_cat, mutant)
    if not any(dropped_as in m and "as_is" in m for m in mutant.failed):
        print("FAIL as_is drop did not name", dropped_as, "got", mutant.failed)
        return 1
    print(f"ok as_is-drop names {dropped_as}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
