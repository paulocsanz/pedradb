#!/usr/bin/env python3
"""RFC-0166 P2.4: catalog accounting freeze.

Live catalog must classify every pair as proof or campaign. Mutating an
l28_* pair to object=proof, or a D1/R1/T1/C1 id to campaign, must FAIL
naming that id.
"""

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
    with redirect_stdout(io.StringIO()):
        pf.check_proof_vs_campaign(ROOT, catalog, live)
    if live.failed:
        print("FAIL live proof-vs-campaign:", live.failed)
        return 1
    print("ok live proof vs campaign")

    l28 = next((p for p in catalog["pairs"] if (p.get("id") or "").startswith("l28_")), None)
    if l28 is None:
        print("FAIL: no l28_* campaign pair")
        return 1
    mutant = copy.deepcopy(catalog)
    for p in mutant["pairs"]:
        if p.get("id") == l28["id"]:
            p["object"] = "proof"
            break
    report = pf.Report()
    with redirect_stdout(io.StringIO()):
        pf.check_proof_vs_campaign(ROOT, mutant, report)
    if not any(l28["id"] in m and "campaign" in m for m in report.failed):
        print("FAIL l28-as-proof did not name", l28["id"], "got", report.failed)
        return 1
    print(f"ok l28-as-proof names {l28['id']}")

    prop = next((p for p in catalog["pairs"] if p.get("id") == "d1_modelo"), None)
    if prop is None:
        print("FAIL: missing d1_modelo proof object")
        return 1
    mutant = copy.deepcopy(catalog)
    for p in mutant["pairs"]:
        if p.get("id") == "d1_modelo":
            p["object"] = "campaign"
            break
    report = pf.Report()
    with redirect_stdout(io.StringIO()):
        pf.check_proof_vs_campaign(ROOT, mutant, report)
    if not any("d1_modelo" in m and "proof object" in m for m in report.failed):
        print("FAIL d1_modelo-as-campaign did not name the id, got", report.failed)
        return 1
    print("ok d1_modelo-as-campaign names d1_modelo")
    return 0


if __name__ == "__main__":
    sys.exit(main())
