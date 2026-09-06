#!/usr/bin/env python3
"""RFC-0170 P0.1 + P1.6: atom without atom_reason fails; stale reason fails."""

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


def _run(catalog: dict) -> pf.Report:
    report = pf.Report()
    with redirect_stdout(io.StringIO()):
        pf.check_proof_depth(ROOT, catalog, report)
    return report


def main() -> int:
    catalog = json.loads((ROOT / "scripts/formal/catalog.json").read_text(encoding="utf-8"))
    live = _run(catalog)
    live_atom = [m for m in live.failed if "atom_reason" in m]
    if live_atom:
        print("FAIL live catalog already missing atom_reason:", live_atom[:3])
        return 1
    print("ok live proof_depth")

    mutant = copy.deepcopy(catalog)
    mutant["pairs"].append(
        {
            "id": "rfc0170_atom_probe",
            "twin_kind": "atom",
            "atom": "bump_non_ff",
            "kernel": "crates/pedradb-core/src/prefix.rs",
            "twin": "crates/pedradb-core/verus/prefix_exclusive_end.rs",
            "entry": "prefix_exclusive_end",
        }
    )
    hits = [m for m in _run(mutant).failed if "rfc0170_atom_probe" in m and "atom_reason" in m]
    if not hits:
        print("FAIL missing atom_reason did not fail")
        return 1
    print("ok missing atom_reason names rfc0170_atom_probe")

    stale = copy.deepcopy(catalog)
    stale["pairs"].append(
        {
            "id": "rfc0170_stale_atom",
            "twin_kind": "atom",
            "atom": "bump_non_ff",
            "kernel": "crates/pedradb-core/src/prefix.rs",
            "twin": "crates/pedradb-core/verus/prefix_exclusive_end.rs",
            "entry": "prefix_exclusive_end",
            "atom_reason": {
                "date": "2026-01-01",
                "why": "stale on purpose",
            },
        }
    )
    hits = [
        m
        for m in _run(stale).failed
        if "rfc0170_stale_atom" in m and "30 days" in m
    ]
    if not hits:
        print("FAIL stale atom_reason did not fail")
        return 1
    print("ok stale atom_reason names rfc0170_stale_atom")
    return 0


if __name__ == "__main__":
    sys.exit(main())
