#!/usr/bin/env python3
"""RFC-0218 — seL4-ladder coverage metric (read-only, repeatable).

ONE number for the gradual seL4-class advance, computed purely from the
registry/catalog so it cannot drift from the ladder floors:

  seL4-cov% = distinct catalog pairs carrying a REGISTERED ∀-theorem rung
              (close or atom row in scripts/ratchet/close_proofs.tsv)
              ÷ total catalog pairs in scripts/formal/catalog.json

Pinned definition (RFC-0218):

- the surface is the WHOLE declared verification surface — every catalog
  pair, campaign-born (`l28_*`) included. Campaign-born pairs are product
  properties; only their REGISTERED theorem counts here, never a campaign
  execution (campaign ≠ ∀π stays in the TCB);
- the numerator is the set of ids with a close or atom row (the ∀-theorem
  rungs). `count` rows (RFC-0199 work credit) never count — they are
  orthogonal to the ladder;
- a pair with both close and atom rows counts once (the atom subsumes).

Cross-checks (exit 1 on mismatch): every registry id must resolve in the
catalog; registered atom/close row counts must EQUAL the live floors in
scripts/ratchet/proof_depth.tsv, so this metric and the depth-floor gate
agree by construction.
"""

from __future__ import annotations

import json
import sys
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REGISTRY = REPO / "scripts" / "ratchet" / "close_proofs.tsv"
FLOORS = REPO / "scripts" / "ratchet" / "proof_depth.tsv"
CATALOG = REPO / "scripts" / "formal" / "catalog.json"


def parse_floors(text: str) -> dict[str, int]:
    out: dict[str, int] = {}
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        k, _, v = line.partition(" ")
        out[k] = int(v.strip())
    return out


def main() -> int:
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    campaign_prefixes = tuple(catalog.get("campaign_prefixes", ()))
    pairs = catalog["pairs"]
    ids = [p["id"] for p in pairs]
    idset = set(ids)
    if len(ids) != len(idset):
        dupes = [i for i, n in Counter(ids).items() if n > 1]
        print(f"FAIL sel4-coverage: duplicate catalog ids {dupes}", file=sys.stderr)
        return 1

    kinds: dict[str, set[str]] = {"close": set(), "atom": set(), "count": set()}
    for line in REGISTRY.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) != 5:
            print(f"FAIL sel4-coverage: malformed registry row {line!r}", file=sys.stderr)
            return 1
        kind, cid, _, _, _ = parts
        if kind not in kinds:
            print(f"FAIL sel4-coverage: unknown registry kind {kind!r}", file=sys.stderr)
            return 1
        if cid.startswith("catalog:"):
            cid = cid[len("catalog:") :]
        if cid not in idset:
            print(f"FAIL sel4-coverage: registry id {cid!r} not in catalog", file=sys.stderr)
            return 1
        kinds[kind].add(cid)

    floors = parse_floors(FLOORS.read_text(encoding="utf-8"))
    errs = []
    if len(kinds["atom"]) != floors["floor_atom"]:
        errs.append(
            f"registered atom ids {len(kinds['atom'])} != floor_atom {floors['floor_atom']}"
        )
    if len(kinds["close"]) != floors["floor_close"]:
        errs.append(
            f"registered close ids {len(kinds['close'])} != floor_close {floors['floor_close']}"
        )
    if errs:
        print("FAIL sel4-coverage: " + "; ".join(errs), file=sys.stderr)
        return 1

    covered = kinds["close"] | kinds["atom"]
    both = kinds["close"] & kinds["atom"]
    campaign_ids = {i for i in idset if i.startswith(campaign_prefixes)}
    pending = idset - covered
    pct = 100.0 * len(covered) / len(idset)

    print("seL4-ladder coverage (registered ∀-theorem rungs / catalog pairs)")
    print(f"  catalog pairs: {len(idset)} (campaign-born {':'.join(campaign_prefixes) or '-'}: "
          f"{len(campaign_ids)})")
    print(f"  atom rungs: {len(kinds['atom'])} (floor_atom {floors['floor_atom']})")
    print(f"  close rungs: {len(kinds['close'])} (floor_close {floors['floor_close']}"
          f"{f', {len(both)} also atom' if both else ''})")
    print(f"  count credit ids (orthogonal, not counted): {len(kinds['count'])}")
    print(f"  covered: {len(covered)}   pending: {len(pending)}")
    print(f"  sel4_coverage = {len(covered)}/{len(idset)} = {pct:.2f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
