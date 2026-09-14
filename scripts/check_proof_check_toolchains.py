#!/usr/bin/env python3
"""RFC-0222 P1.1 — proof-check.yml pins formal toolchains and invokes
``lean_extracts.sh --required`` plus the ``aeneas_*.sh --required`` corpus.

Skip→exit 0 is the hole this gate closes: CI without the invocations can
never claim Lean/Aeneas proved anything. The pins live in the shipped
yaml (elan/lean 4.31.0, Aeneas ``daa85d7``, Charon ``340b1af``) matching
``formal/aeneas/PINS.md``.
"""
from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
WORKFLOW = REPO / ".github/workflows/proof-check.yml"
PINS = {
    "lean": "leanprover/lean4:v4.31.0",
    "aeneas": "daa85d7e89400fa978be83fedbc7e475a83f0889",
    "charon": "340b1af4df92608d0911fc2ba26eef3fd3a30ab4",
}


def check(text: str) -> list[str]:
    errs: list[str] = []
    if "lean_extracts.sh --required" not in text:
        errs.append("proof-check.yml must invoke lean_extracts.sh --required")
    if "aeneas_*.sh" not in text or "--required" not in text:
        errs.append("proof-check.yml must invoke scripts/aeneas_*.sh --required")
    for name, pin in PINS.items():
        if pin not in text:
            errs.append(f"proof-check.yml must pin {name} as {pin}")
    return errs


def selftest() -> int:
    text = WORKFLOW.read_text(encoding="utf-8")
    base = check(text)
    if base:
        print("SELFTEST proof-check-toolchains: workflow already inconsistent:")
        for e in base:
            print(f"  {e}")
        return 1
    caught = 0
    total = 3
    if check(text.replace("lean_extracts.sh --required", "lean_extracts.sh")):
        print("SELFTEST proof-check-toolchains: caught=lean_extracts-skip")
        caught += 1
    else:
        print("SELFTEST proof-check-toolchains: MISSED lean_extracts-skip")
    if check(text.replace("aeneas_*.sh", "aeneas_skip.sh")):
        print("SELFTEST proof-check-toolchains: caught=aeneas-glob-removed")
        caught += 1
    else:
        print("SELFTEST proof-check-toolchains: MISSED aeneas-glob-removed")
    if check(text.replace(PINS["aeneas"], "deadbeef" * 5)):
        print("SELFTEST proof-check-toolchains: caught=aeneas-pin-stripped")
        caught += 1
    else:
        print("SELFTEST proof-check-toolchains: MISSED aeneas-pin-stripped")
    print(f"SELFTEST proof-check-toolchains: {caught}/{total} sabotages caught")
    return 0 if caught == total else 1


def main() -> int:
    if "--selftest" in sys.argv:
        return selftest()
    errs = check(WORKFLOW.read_text(encoding="utf-8"))
    if errs:
        for e in errs:
            print(f"FAIL  {e}")
        print("GATE proof-check-toolchains: RED")
        return 1
    print("GATE proof-check-toolchains: GREEN — pins + --required invocations present")
    return 0


if __name__ == "__main__":
    sys.exit(main())
