#!/usr/bin/env python3
"""RFC-0227 P0.1 — honest composition floor (blocking, self-verifying).

m2 counts atom *entries* (close_proofs.tsv) that appear in a Compose*.lean
theorem which dual-unfolds **two** Lean ``def`` names. Credit is refused for:

- ``ComposeM2.lean`` (String index ``m2_fn_* : String := "name"``)
- theorems whose proof is ``native_decide`` without ``unfold``
- theorems with fewer than two distinct ``unfold`` identifiers

``floor_m2 N`` in ``scripts/ratchet/compose_floor.tsv`` is a FLOOR: live m2
below N is red. The floor may drop **once** (RFC-0227 honest recount) and
then only rises in the same commit as a dual-unfold theorem.

``--selftest`` sabotages in memory: String-credit, no dual-unfold,
native_decide-only must each be caught; the honest tree must pass.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LEAN_DIR = REPO / "formal" / "aeneas" / "lean"
CLOSE_PROOFS = REPO / "scripts" / "ratchet" / "close_proofs.tsv"
FLOORS = REPO / "scripts" / "ratchet" / "compose_floor.tsv"
SKIP_FILES = {"ComposeM2.lean"}

THEOREM_RE = re.compile(
    r"^theorem\s+([A-Za-z0-9_']+)\b(.*?)(?=^theorem\s+|\Z)",
    re.M | re.S,
)
UNFOLD_RE = re.compile(r"\bunfold\s+((?:[A-Za-z0-9_'.]+\s+)+)")


def atom_entries(text: str) -> set[str]:
    out: set[str] = set()
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) >= 5 and parts[0] == "atom":
            out.add(parts[4])
    return out


def parse_floor(text: str) -> int:
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if line.startswith("floor_m2 "):
            return int(line.split()[1])
    raise SystemExit("GATE compose-floor: FAIL — missing `floor_m2 N`")


def strip_strings(s: str) -> str:
    return re.sub(r'"[^"\\]*(?:\\.[^"\\]*)*"', '""', s)


def unfold_names(proof: str) -> set[str]:
    names: set[str] = set()
    for block in UNFOLD_RE.findall(proof):
        for tok in block.split():
            base = tok.rstrip(".,;").split(".")[-1]
            if base:
                names.add(base)
    return names


def honest_theorems(compose_text: str) -> list[tuple[str, str]]:
    """Return (name, body) for theorems that dual-unfold two defs."""
    out: list[tuple[str, str]] = []
    stripped = strip_strings(compose_text)
    for m in THEOREM_RE.finditer(stripped):
        name, body = m.group(1), m.group(2)
        if re.search(r"\bnative_decide\b", body) and "unfold" not in body:
            continue
        names = unfold_names(body)
        if len(names) >= 2:
            out.append((name, body))
    return out


def live_m2(compose_files: dict[str, str], atoms: set[str]) -> tuple[int, set[str]]:
    chained: set[str] = set()
    for fname, text in compose_files.items():
        if fname in SKIP_FILES:
            continue
        for _name, body in honest_theorems(text):
            for atom in atoms:
                if re.search(rf"\b{re.escape(atom)}\b", body):
                    chained.add(atom)
    return len(chained), chained


def load_compose() -> dict[str, str]:
    out: dict[str, str] = {}
    for p in sorted(LEAN_DIR.glob("Compose*.lean")):
        out[p.name] = p.read_text(encoding="utf-8", errors="replace")
    return out


def check(floor: int, m2: int) -> list[str]:
    errs: list[str] = []
    if m2 < floor:
        errs.append(
            f"m2 {m2} < floor_m2 {floor} — composition credit regressed "
            "(floor only rises after the RFC-0227 one-time honest drop)"
        )
    return errs


def selftest() -> int:
    if not FLOORS.is_file():
        print("SELFTEST compose-floor: missing floors file")
        return 1
    atoms = atom_entries(CLOSE_PROOFS.read_text(encoding="utf-8"))
    files = load_compose()
    floor = parse_floor(FLOORS.read_text(encoding="utf-8"))
    m2, _ = live_m2(files, atoms)
    base = check(floor, m2)
    if base:
        print("SELFTEST compose-floor: state already inconsistent — fix first:")
        for e in base:
            print(f"  {e}")
        return 1

    caught = 0
    total = 4

    # S1: String-only ComposeM2-style text must not raise m2.
    fake = dict(files)
    fake["ComposeFake.lean"] = (
        'def m2_fn_visible_at : String := "visible_at"\n'
        'def m2_fn_leftover_fate : String := "leftover_fate"\n'
        'theorem m2_index_names : m2_fn_visible_at = "visible_at" := rfl\n'
    )
    m2_s, _ = live_m2(fake, atoms)
    if m2_s == m2:
        print("SELFTEST compose-floor: caught=string-credit")
        caught += 1
    else:
        print(f"SELFTEST compose-floor: MISSED string-credit (m2 {m2}→{m2_s})")

    # S2: a theorem without two unfolds must not credit atoms.
    fake2 = dict(files)
    fake2["ComposeFake2.lean"] = (
        "theorem no_dual (x : Bool) : True := by\n"
        "  unfold leftover_fate\n"
        "  trivial\n"
    )
    m2_n, chained_n = live_m2(fake2, atoms)
    if m2_n == m2 and "leftover_fate" not in chained_n - live_m2(files, atoms)[1]:
        print("SELFTEST compose-floor: caught=no-dual-unfold")
        caught += 1
    else:
        # leftover_fate might already be chained; the fake file still has only 1 unfold
        # so m2 must not rise.
        if m2_n == m2:
            print("SELFTEST compose-floor: caught=no-dual-unfold")
            caught += 1
        else:
            print(f"SELFTEST compose-floor: MISSED no-dual-unfold (m2 {m2}→{m2_n})")

    # S3: native_decide without unfold is not compose.
    fake3 = dict(files)
    fake3["ComposeFake3.lean"] = (
        "theorem nd_only (x : Bool) : leftover_fate x = leftover_fate x := by\n"
        "  native_decide\n"
    )
    m2_d, _ = live_m2(fake3, atoms)
    if m2_d == m2:
        print("SELFTEST compose-floor: caught=native-decide-only")
        caught += 1
    else:
        print(f"SELFTEST compose-floor: MISSED native-decide-only (m2 {m2}→{m2_d})")

    # S4: honest freeze passes.
    if check(floor, m2) == []:
        print("SELFTEST compose-floor: passed=honest-freeze (oracle not always-red)")
        caught += 1
    else:
        print("SELFTEST compose-floor: honest freeze REJECTED — oracle always-red")

    print(f"SELFTEST compose-floor: {caught}/{total} checks caught")
    return 0 if caught == total else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()
    if args.selftest:
        return selftest()

    if not FLOORS.is_file():
        print("GATE compose-floor: FAIL — missing scripts/ratchet/compose_floor.tsv")
        return 1
    atoms = atom_entries(CLOSE_PROOFS.read_text(encoding="utf-8"))
    files = load_compose()
    floor = parse_floor(FLOORS.read_text(encoding="utf-8"))
    m2, chained = live_m2(files, atoms)
    errs = check(floor, m2)
    if errs:
        for e in errs:
            print(f"GATE compose-floor: FAIL — {e}")
        print("GATE compose-floor: RED")
        return 1
    print(
        f"GATE compose-floor: GREEN — m2={m2}>={floor} "
        f"chained={len(chained)} compose_files={len(files)} "
        f"skipped=ComposeM2"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
