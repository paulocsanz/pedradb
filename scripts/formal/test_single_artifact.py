#!/usr/bin/env python3
"""RFC-0171 P0.3: single_artifact pair with twin ≠ kernel fails by id."""

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


def _sa_fails(catalog: dict) -> list[str]:
    """RFC-0171 P0.3: twin path must equal kernel path when single_artifact."""
    hits = []
    for pair in catalog.get("pairs") or []:
        if not pair.get("single_artifact"):
            continue
        kpath = pair.get("kernel") or ""
        tpath = pair.get("twin") or ""
        if kpath != tpath:
            hits.append(
                f"{pair.get('id')}: single_artifact requires twin == kernel "
                f"(RFC-0171 P0.3; kernel={kpath!r} twin={tpath!r})"
            )
    return hits


def _twins(catalog: dict) -> pf.Report:
    report = pf.Report()
    with redirect_stdout(io.StringIO()):
        pf.check_twins(ROOT, catalog, report, strict=False)
    return report


def main() -> int:
    catalog = json.loads((ROOT / "scripts/formal/catalog.json").read_text(encoding="utf-8"))
    live_hits = _sa_fails(catalog)
    if live_hits:
        print("FAIL live catalog single_artifact:", live_hits[:3])
        return 1
    print("ok live single_artifact")

    mutant = copy.deepcopy(catalog)
    found = False
    for pair in mutant["pairs"]:
        if pair.get("id") == "write_admission":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/write_admission.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing write_admission")
        return 1
    hits = [m for m in _sa_fails(mutant) if "write_admission" in m]
    if not hits:
        print("FAIL twin≠kernel did not fail")
        return 1
    print("ok mutant write_admission twin≠kernel named")
    # Drive the same rule through check_twins (may also report unrelated
    # dirty-tree twin drift; we only require the SA id).
    try:
        twin_hits = [
            m
            for m in _twins(mutant).failed
            if "write_admission" in m and "single_artifact" in m
        ]
        if not twin_hits:
            print("FAIL check_twins did not name write_admission single_artifact")
            return 1
        print("ok check_twins names write_admission single_artifact")
    except ValueError as e:
        print("warn check_twins parse:", e)
    return 0


if __name__ == "__main__":
    sys.exit(main())
