#!/usr/bin/env python3
"""RFC-0203 P0.1 — twin-contract gate (blocking, self-verifying).

Every registered `count` row in `scripts/ratchet/close_proofs.tsv` must be
bound by a contract in the machine-emitted `scripts/ratchet/twin_contracts.tsv`:

- coverage: no count row without a contract; no contract without a count
  row; theorem/lean_file must match the registry row;
- twin: the twin test exists on disk and drives the named production fn
  (a twin that mocks the unit under test is RED);
- fn: the production fn exists in the crate sources;
- annotation: the named work bound exists — `*_step_work` defs in the
  mechanically derived CountDerived.lean for enrolled pairs, or the
  registered hand theorem (WorkIo / BloomCount mirrors) for the rest;
- sync: CountDerived.lean matches what the derive tool emits for the
  current extracts;
- machine-maintained: the on-disk TSV is byte-identical to what the
  derive tool emits for the current registry (hand-edited contract = RED).

`--selftest` proves redness in memory: a dropped contract, a twin that no
longer drives the fn, a contract for an unregistered pair, a stale
annotation name, a hand-edited TSV, and a stale CountDerived.lean must
each be caught; the healthy state must pass.
"""

from __future__ import annotations

import argparse
import importlib.util
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REGISTRY = REPO / "scripts" / "ratchet" / "close_proofs.tsv"
CONTRACTS = REPO / "scripts" / "ratchet" / "twin_contracts.tsv"
DERIVE_TOOL = REPO / "scripts" / "ratchet" / "derive_count_annotations.py"
FIELDS = ("catalog_id", "theorem", "lean_file", "twin_test", "prod_fn", "annotation", "annotation_file")

# the mechanically derived annotation file (emit side of the derive tool)
COUNT_DERIVED = "formal/aeneas/lean/CountDerived.lean"


def load_derive_tool():
    spec = importlib.util.spec_from_file_location("derive_count_annotations", DERIVE_TOOL)
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(mod)
    return mod


def parse_registry(text: str) -> list[dict[str, str]]:
    rows = []
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) == 5 and parts[0] == "count":
            rows.append(dict(zip(("kind", "catalog_id", "theorem", "lean_file", "entry"), parts)))
    return rows


def parse_contracts(text: str) -> list[dict[str, str]]:
    rows = []
    for line in text.splitlines():
        stripped = line.split("#", 1)[0].strip("\t ").rstrip()
        if not stripped:
            continue
        parts = stripped.split("\t")
        if len(parts) != 7:
            raise SystemExit(f"GATE twin-contracts: FAIL — twin_contracts.tsv row needs 7 tab-separated fields: {line!r}")
        rows.append(dict(zip(FIELDS, parts)))
    seen: set[str] = set()
    for row in rows:
        if row["catalog_id"] in seen:
            raise SystemExit(
                f"GATE twin-contracts: FAIL — duplicate contract row {row['catalog_id']} (one contract per count row)"
            )
        seen.add(row["catalog_id"])
    return rows


def crate_fn_names(read) -> set[str]:
    """Every `fn <name>` defined under crates/ (production fn exists)."""
    names: set[str] = set()
    for path in sorted((REPO / "crates").rglob("*.rs")):
        text = read(str(path.relative_to(REPO)))
        if text is None:
            continue
        names.update(re.findall(r"\bfn\s+(\w+)", text))
    return names


def check(
    contracts: list[dict[str, str]],
    count_rows: list[dict[str, str]],
    read,  # rel path -> text | None  (disk reader, or selftest override)
    rendered_tsv: str,
    on_disk_tsv: str,
    fns: set[str],
) -> list[str]:
    errs: list[str] = []
    by_pair = {c["catalog_id"]: c for c in contracts}
    rows_by_pair = {r["catalog_id"]: r for r in count_rows}

    for row in count_rows:
        tag = f"count {row['theorem']}"
        c = by_pair.get(row["catalog_id"])
        if c is None:
            errs.append(f"{tag}: no twin contract for {row['catalog_id']} — every count row needs one")
            continue
        if c["theorem"] != row["theorem"] or c["lean_file"] != row["lean_file"]:
            errs.append(f"{tag}: contract out of sync with the registry row ({c['theorem']} @ {c['lean_file']})")
        if not re.search(rf"\b{re.escape(c['prod_fn'])}\b", read(c["twin_test"]) or ""):
            errs.append(
                f"{tag}: twin {c['twin_test']} does not drive prod fn `{c['prod_fn']}` — "
                "the twin must exercise the production fn, not a mock"
            )
        if c["prod_fn"] not in fns:
            errs.append(f"{tag}: prod fn `{c['prod_fn']}` not defined under crates/ — contract names a fn that does not exist")
        ann_text = read(c["annotation_file"])
        if ann_text is None:
            errs.append(f"{tag}: annotation file {c['annotation_file']} does not exist")
        else:
            for ann in c["annotation"].split():
                if c["annotation_file"] == COUNT_DERIVED:
                    if not re.search(rf"^def {re.escape(ann)}\b", ann_text, re.MULTILINE):
                        errs.append(f"{tag}: no `def {ann}` in {c['annotation_file']} — stale derived annotation")
                elif not re.search(rf"\btheorem\s+{re.escape(ann)}\b", ann_text):
                    errs.append(f"{tag}: no `theorem {ann}` in {c['annotation_file']} — stale hand annotation")

    for cid in by_pair.keys() - rows_by_pair.keys():
        errs.append(f"contract {cid}: no registered count row for this pair — stale contract")

    if on_disk_tsv != rendered_tsv:
        errs.append(
            "twin_contracts.tsv is not what the derive tool emits for the current registry — "
            "hand-edited or stale; regenerate with scripts/ratchet/derive_count_annotations.py"
        )
    return errs


def derived_sync_errs(read) -> list[str]:
    tool = load_derive_tool()
    expected = tool.render(tool.derive(tool.LEAN_DIR_DEFAULT))
    on_disk = read(COUNT_DERIVED)
    if on_disk != expected:
        return [
            "CountDerived.lean stale vs the extracts — regenerate with "
            "scripts/ratchet/derive_count_annotations.py (lean gate also checks this)"
        ]
    return []


def disk_read(rel: str) -> str | None:
    p = REPO / rel
    return p.read_text(encoding="utf-8") if p.is_file() else None


def selftest() -> int:
    tool = load_derive_tool()
    count_rows = parse_registry(REGISTRY.read_text(encoding="utf-8"))
    on_disk_tsv = CONTRACTS.read_text(encoding="utf-8")
    contracts = parse_contracts(on_disk_tsv)
    rendered = tool.render_contracts(tool.count_registry_rows())
    if rendered != on_disk_tsv:
        print("SELFTEST twin-contracts: TSV not machine-emitted — regenerate first:")
        print("  python3 scripts/ratchet/derive_count_annotations.py")
        return 1
    fns = crate_fn_names(disk_read)
    base = check(contracts, count_rows, disk_read, rendered, on_disk_tsv, fns) + derived_sync_errs(disk_read)
    if base:
        print("SELFTEST twin-contracts: state already inconsistent — fix first:")
        for e in base:
            print(f"  {e}")
        return 1

    caught = 0
    total = 6

    # S1: a count row lost its contract.
    dropped = [c for c in contracts if c["catalog_id"] != "catalog:bloom_may_contain"]
    if check(dropped, count_rows, disk_read, rendered, on_disk_tsv, fns):
        print("SELFTEST twin-contracts: caught=missing-contract")
        caught += 1
    else:
        print("SELFTEST twin-contracts: MISSED missing contract")

    # S2: twin no longer drives the named production fn (mock twin swap).
    mocked = [dict(c) for c in contracts]
    for c in mocked:
        if c["catalog_id"] == "catalog:lsm_compact":
            c["prod_fn"] = "point_get_probes"  # exists in crates/, never called by the lsm twin
    if check(mocked, count_rows, disk_read, rendered, on_disk_tsv, fns):
        print("SELFTEST twin-contracts: caught=mock-twin")
        caught += 1
    else:
        print("SELFTEST twin-contracts: MISSED mock twin")

    # S3: contract for a pair that never registered a count row.
    stale = contracts + [dict(contracts[0], catalog_id="catalog:never_registered")]
    if check(stale, count_rows, disk_read, rendered, on_disk_tsv, fns):
        print("SELFTEST twin-contracts: caught=unregistered-contract")
        caught += 1
    else:
        print("SELFTEST twin-contracts: MISSED unregistered contract")

    # S4: annotation name that no longer exists in its annotation file.
    stale_ann = [dict(c) for c in contracts]
    for c in stale_ann:
        if c["catalog_id"] == "catalog:scan_guard":
            c["annotation"] = "no_such_step_work_anywhere"
    if check(stale_ann, count_rows, disk_read, rendered, on_disk_tsv, fns):
        print("SELFTEST twin-contracts: caught=stale-annotation")
        caught += 1
    else:
        print("SELFTEST twin-contracts: MISSED stale annotation")

    # S5: hand-edited TSV (differs from what the tool emits for this registry).
    tampered_tsv = on_disk_tsv.replace("lsm_compact_work_bound", "lsm_compact_hand_edit", 1)
    if check(parse_contracts(tampered_tsv), count_rows, disk_read, rendered, tampered_tsv, fns):
        print("SELFTEST twin-contracts: caught=hand-edited-tsv")
        caught += 1
    else:
        print("SELFTEST twin-contracts: MISSED hand-edited TSV")

    # S6: CountDerived.lean stale against the extracts.
    def tampered_read(rel: str):
        text = disk_read(rel)
        if rel == COUNT_DERIVED and text is not None:
            return text + "\n-- tampered\n"
        return text

    if derived_sync_errs(tampered_read):
        print("SELFTEST twin-contracts: caught=stale-count-derived")
        caught += 1
    else:
        print("SELFTEST twin-contracts: MISSED stale CountDerived.lean")

    print(f"SELFTEST twin-contracts: {caught}/{total} sabotages caught")
    return 0 if caught == total else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--selftest", action="store_true", help="prove all red directions")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    tool = load_derive_tool()
    count_rows = parse_registry(REGISTRY.read_text(encoding="utf-8"))
    on_disk_tsv = CONTRACTS.read_text(encoding="utf-8") if CONTRACTS.is_file() else ""
    contracts = parse_contracts(on_disk_tsv)
    try:
        rendered = tool.render_contracts(tool.count_registry_rows())
    except SystemExit as e:
        print(f"GATE twin-contracts: FAIL — {e}")
        print("GATE twin-contracts: RED")
        return 1
    errs = check(contracts, count_rows, disk_read, rendered, on_disk_tsv, crate_fn_names(disk_read))
    errs += derived_sync_errs(disk_read)
    if errs:
        for e in errs:
            print(f"GATE twin-contracts: FAIL — {e}")
        print("GATE twin-contracts: RED")
        return 1
    enrolled = sum(1 for c in contracts if c["annotation_file"] == COUNT_DERIVED)
    hand = len(contracts) - enrolled
    print(
        f"GATE twin-contracts: GREEN — {len(contracts)}/{len(count_rows)} count rows bound "
        f"(twin drives prod fn + annotation present; {enrolled} derived / {hand} hand), "
        "TSV machine-emitted, CountDerived.lean in sync"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
