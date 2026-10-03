#!/usr/bin/env python3
"""Public-mirror hygiene gate.

Fails when the public tree carries internal scaffolding or references to
documents that live only in the development tree. Run by CI on every push:

    python3 scripts/check_public_hygiene.py

Exit 0 = clean; exit 1 = violations printed.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Directories that must never ship in the public mirror.
FORBIDDEN_DIRS = [
    ".agents",
    ".grok",
    "docs/rfc",
    "docs/audits",
    "docs/reports",
    "docs/research",
    "docs/references",
    "docs/runbooks",
    "docs/formal",
    "findings",
]

# Cross-repo names from the dev environment must not leak as file names.
FORBIDDEN_NAME_STEMS = ("caixote", "fonte", "janela")

# Markdown links pointing at internal-only trees (relative to any doc).
INTERNAL_LINK_RE = re.compile(
    r"\]\((?:\.\./)*(?:docs/)?"
    r"(?:rfc|audits|reports|research|references|runbooks)/[^)]+\)"
)

# Sibling working docs that were removed from the mirror.
REMOVED_SIBLINGS = (
    "montanhadb",
    "montanha-layering-dcs-on-store",
    "distribution-design",
    "engine-landscape-and-ideal-path",
    "fdb-limitations-analysis",
    "dst-seams",
    "live-leadership-and-patroni-shaped-ha",
    "TRAJETORIA",
    "apply-and-raft",
    "positioning",
)
SIBLING_LINK_RE = re.compile(
    r"\]\((?:\.\./)*(?:docs/)?(" + "|".join(REMOVED_SIBLINGS) + r")\.md\)"
)


def main() -> int:
    bad = False

    for d in FORBIDDEN_DIRS:
        if (ROOT / d).is_dir():
            print(f"HYGIENE FAIL: internal directory shipped: {d}/")
            bad = True

    for p in ROOT.rglob("*"):
        if not p.is_file() or ".git" in p.parts:
            continue
        stem = p.name.lower()
        if any(s in stem for s in FORBIDDEN_NAME_STEMS):
            print(f"HYGIENE FAIL: cross-repo file name leaked: {p.relative_to(ROOT)}")
            bad = True

    for p in list(ROOT.glob("*.md")) + list((ROOT / "docs").glob("*.md")):
        text = p.read_text(encoding="utf-8", errors="replace")
        rel = p.relative_to(ROOT)
        for m in INTERNAL_LINK_RE.finditer(text):
            print(f"HYGIENE FAIL: {rel} links internal-only path: {m.group(0)}")
            bad = True
        for m in SIBLING_LINK_RE.finditer(text):
            print(f"HYGIENE FAIL: {rel} links removed sibling doc: {m.group(0)}")
            bad = True

    if not bad:
        print("hygiene: clean (no internal scaffolding, no dangling internal links)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
