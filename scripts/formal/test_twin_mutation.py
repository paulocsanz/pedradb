#!/usr/bin/env python3
"""RFC-0166 P2.5: meta-mutation of pedra_formal twin checks.

Clone a close twin, inject (1) syntactic drift — the exec entry fn is
renamed away — and (2) semantic drift — the twin keeps the fn but drops
kernel decision tokens — and require FAIL that names the pair id. Today
this was done once by hand (findings/2026-09-04-clone-as-is-blindspot);
it is now a test.
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


def _close_pair(catalog: dict) -> dict:
    for p in catalog["pairs"]:
        if p.get("twin_kind") != "close" or p.get("status") == "absent":
            continue
        entry = p.get("token_src") or p.get("entry")
        if not entry:
            continue
        if (ROOT / p["twin"]).is_file() and (ROOT / p["kernel"]).is_file():
            return p
    raise SystemExit("FAIL: no close pair with on-disk kernel+twin")


def _run_twins(catalog: dict) -> pf.Report:
    report = pf.Report()
    with redirect_stdout(io.StringIO()):
        pf.check_twins(ROOT, catalog, report, strict=False)
    return report


def main() -> int:
    catalog = json.loads((ROOT / "scripts/formal/catalog.json").read_text(encoding="utf-8"))
    pair = _close_pair(catalog)
    pid = pair["id"]
    entry = pair.get("token_src") or pair["entry"]
    print(f"mutating close pair {pid} entry {entry}")

    live = _run_twins(catalog)
    live_this = [m for m in live.failed if m.startswith(pid + ":")]
    if live_this:
        print("FAIL live twin already broken for", pid, live_this)
        return 1
    print(f"ok live twin for {pid}")

    orig_twin = (ROOT / pair["twin"]).read_text(encoding="utf-8")
    sibling = (ROOT / pair["twin"]).with_name(
        (ROOT / pair["twin"]).stem + ".__mut.rs"
    )

    def check_mutant(text: str, needle: str) -> list[str]:
        mutant_cat = copy.deepcopy(catalog)
        try:
            sibling.write_text(text, encoding="utf-8")
            for p in mutant_cat["pairs"]:
                if p.get("id") == pid:
                    p["twin"] = str(sibling.relative_to(ROOT))
                    break
            return [m for m in _run_twins(mutant_cat).failed if pid in m and needle in m]
        finally:
            if sibling.exists():
                sibling.unlink()

    # (1) syntactic: rename the exec entry so the close twin "loses" it.
    renamed = orig_twin.replace(f"fn {entry}", f"fn {entry}_gone", 1)
    if renamed == orig_twin:
        print(f"FAIL could not rename fn {entry} in {pair['twin']}")
        return 1
    hits = check_mutant(renamed, "missing exec fn")
    if not hits:
        print("FAIL rename-entry did not report missing exec fn")
        return 1
    print(f"ok rename-entry names {pid}")

    # (2) semantic: keep the fn, empty the body so kernel tokens vanish.
    stub_only = f"use vstd::prelude::*;\nverus! {{\npub fn {entry}() {{}}\n}}\n"
    hits = check_mutant(stub_only, "missing tokens")
    if not hits:
        # Some kernels have empty/token-free bodies; accept any named FAIL.
        hits = check_mutant(stub_only, pid + ":")
        if not hits:
            print("FAIL empty-body did not name", pid)
            return 1
    print(f"ok empty-body names {pid}: {hits[0]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
