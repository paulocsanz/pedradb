#!/usr/bin/env python3
"""RFC-0050 P0.4 seam inventory check (L0/L1/L2 doctrine from RFC-0018).

Exit 0 = every inventory row is L2 (listed + injector + trial_ref resolves to
a real in-tree test) and the site id set matches `SEAM_IDS` in
`crates/pedradb-world/src/coverage.rs`. Any orphan site (missing or
unresolvable trial_ref), broken injector, or SEAM_IDS drift fails CI.

Usage: check_seam_inventory.py [inventory.json]
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CRATES = REPO / "crates"
DEFAULT_INVENTORY = REPO / "scripts" / "seam_inventory_v1.json"
SEAM_IDS_SRC = CRATES / "pedradb-world" / "src" / "coverage.rs"

_FNS_CACHE: dict[str, set[str]] = {}


def seam_ids() -> list[str]:
    text = SEAM_IDS_SRC.read_text()
    m = re.search(r"pub const SEAM_IDS: &\[&str\] = &\[(.*?)\];", text, re.S)
    if not m:
        raise SystemExit(f"SEAM INVENTORY FAIL: cannot parse SEAM_IDS in {SEAM_IDS_SRC}")
    return re.findall(r'"([^"]+)"', m.group(1))


def crate_dir(name: str) -> Path | None:
    for cand in (name, name.replace("_", "-"), name.replace("-", "_")):
        d = CRATES / cand
        if d.is_dir():
            return d
    return None


def crate_test_fns(crate: str) -> set[str]:
    if crate in _FNS_CACHE:
        return _FNS_CACHE[crate]
    fns: set[str] = set()
    d = crate_dir(crate)
    if d is not None:
        for rs in d.rglob("*.rs"):
            if "target" in rs.parts:
                continue
            fns.update(re.findall(r"\bfn\s+(\w+)\s*\(", rs.read_text()))
    _FNS_CACHE[crate] = fns
    return fns


def check(inventory_path: Path) -> int:
    errors: list[str] = []
    data = json.loads(inventory_path.read_text())
    sites = data.get("sites") or []
    if not sites:
        errors.append("inventory has no sites")

    ids: list[str] = []
    for s in sites:
        sid = s.get("id", "")
        ids.append(sid)
        if not sid:
            errors.append("site without id")
            continue
        if not s.get("site"):
            errors.append(f"{sid}: missing 'site'")
        if not s.get("injector"):
            errors.append(f"{sid}: L1 broken - empty injector")
        ref = s.get("trial_ref", "")
        if not ref:
            errors.append(f"{sid}: ORPHAN - no trial_ref (not L2)")
            continue
        parts = ref.split("::")
        if len(parts) < 2 or not parts[0] or not parts[-1]:
            errors.append(f"{sid}: malformed trial_ref '{ref}'")
            continue
        crate, fn = parts[0], parts[-1]
        if crate_dir(crate) is None:
            errors.append(f"{sid}: trial_ref crate '{crate}' not in workspace")
            continue
        if fn not in crate_test_fns(crate):
            errors.append(f"{sid}: trial_ref does not resolve in-tree: '{ref}'")
        else:
            print(f"ok {sid} -> {ref}")

    dupes = sorted({i for i in ids if ids.count(i) > 1})
    if dupes:
        errors.append(f"duplicate site ids: {dupes}")

    want = seam_ids()
    missing = [i for i in want if i not in ids]
    extra = [i for i in ids if i not in want]
    if missing:
        errors.append(f"SEAM_IDS drift - in coverage.rs but not inventory: {missing}")
    if extra:
        errors.append(f"SEAM_IDS drift - in inventory but not coverage.rs: {extra}")

    if errors:
        print()
        for e in errors:
            print(f"FAIL {e}")
        print(f"SEAM INVENTORY FAIL ({inventory_path.name}): {len(errors)} error(s)")
        return 1
    print(
        f"SEAM INVENTORY OK ({inventory_path.name}): "
        f"{len(sites)}/{len(sites)} sites L2, ids match SEAM_IDS"
    )
    return 0


def main() -> int:
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_INVENTORY
    try:
        return check(path)
    except (OSError, json.JSONDecodeError) as exc:
        print(f"SEAM INVENTORY FAIL: cannot read {path}: {exc}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
