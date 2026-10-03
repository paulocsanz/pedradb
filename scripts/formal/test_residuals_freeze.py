#!/usr/bin/env python3
"""RFC-0061: drive the shipped residuals freeze on the real catalog.

Dropping a `never` id from residuals (leaving never_floor) must FAIL naming
that id. The live catalog must pass.
"""

from __future__ import annotations

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
    pf.check_residuals(ROOT, live, catalog)
    if live.failed:
        print("FAIL live catalog:", live.failed)
        return 1
    print("ok live residuals freeze")

    src = json.loads((ROOT / "scripts/formal/residuals.json").read_text(encoding="utf-8"))
    never = [row["id"] for row in src["residuals"] if row["class"] == "never"]
    if not never:
        print("FAIL: catalog has no never rows")
        return 1
    dropped = never[0]
    src["residuals"] = [row for row in src["residuals"] if row["id"] != dropped]
    tmp = ROOT / "scripts/formal/.residuals-drop-never.json"
    try:
        tmp.write_text(json.dumps(src), encoding="utf-8")
        mutant = pf.Report()
        # Expected negative: check_residuals prints FAIL for the drop. Keep
        # live stdout FAIL-free; the mutant still has to *name* the id.
        with redirect_stdout(io.StringIO()):
            pf.check_residuals(ROOT, mutant, catalog, residuals_path=tmp)
    finally:
        if tmp.exists():
            tmp.unlink()
    if not any(dropped in m and "disappeared" in m for m in mutant.failed):
        print("FAIL drop did not name", dropped, "got", mutant.failed)
        return 1
    print(f"ok never-drop names {dropped}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
