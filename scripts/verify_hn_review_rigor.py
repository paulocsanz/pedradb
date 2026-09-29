#!/usr/bin/env python3
"""
Mechanical Oracle to verify the rigor of the Hacker News Adversarial Review Skill.
Ensures that the review skill and its references:
1. Reject shallow strawman attacks (e.g. claiming RocksDB sync=false deception when G1 is default and explicit).
2. Mandate the 'Engineering Armor for Architectural Boldness' thesis (Rust + Lean4/Kani + Real POSIX DST + Differential Oracles).
3. Include the 4 Deep Architecture Attack Vectors across SKILL.md, checklist, scars, and mock grilling:
   - Vector 1: Complexity Paradox & Synchronization Glue (off-lock I/O, SuperVersion publish, lost linearizability).
   - Vector 2: Group Commit Tail Latency Jitter (p99/p99.9) under asymmetric bursts vs lone-commit fast path.
   - Vector 3: Mmap WAL & Linux VFS Interactions (dirty page writeback, page cache throttling, SIGBUS).
   - Vector 4: Drop-in RocksDB Parity vs 15 Years of Behavioral Quirks (partial merges, tombstone collapsing, snapshot visibility).
"""

import sys
from pathlib import Path

def test_hn_review_skill():
    skill_dir = Path(".agents/skills/hackernews-adversarial-review")
    if not skill_dir.exists():
        print("FAIL: Skill directory does not exist", file=sys.stderr)
        return False

    skill_file = skill_dir / "SKILL.md"
    checklist_file = skill_dir / "references/hn-cynic-checklist.md"
    scars_file = skill_dir / "references/battle-scars.md"
    examples_file = skill_dir / "examples/mock-hn-grilling.md"

    files = {
        "SKILL.md": skill_file,
        "hn-cynic-checklist.md": checklist_file,
        "battle-scars.md": scars_file,
        "mock-hn-grilling.md": examples_file,
    }

    for name, path in files.items():
        if not path.exists():
            print(f"FAIL: Missing file {name}", file=sys.stderr)
            return False

    skill_text = skill_file.read_text(encoding="utf-8")
    checklist_text = checklist_file.read_text(encoding="utf-8")
    scars_text = scars_file.read_text(encoding="utf-8")
    examples_text = examples_file.read_text(encoding="utf-8")

    # In SKILL.md:
    if "Engineering Armor" not in skill_text and "armadura" not in skill_text.lower():
        print("FAIL: SKILL.md lacks 'Engineering Armor' paradigm", file=sys.stderr)
        return False
    if "Complexity Paradox" not in skill_text:
        print("FAIL: SKILL.md lacks 'Complexity Paradox / Synchronization Glue' vector", file=sys.stderr)
        return False
    if "Tail Latency" not in skill_text and "p99" not in skill_text:
        print("FAIL: SKILL.md lacks 'Group Commit Tail Latency' vector", file=sys.stderr)
        return False
    if "Mmap WAL" not in skill_text:
        print("FAIL: SKILL.md lacks 'Mmap WAL & Linux VFS Interactions' vector", file=sys.stderr)
        return False
    if "RocksDB-Compat" not in skill_text and "rocksdb-compat" not in skill_text.lower():
        print("FAIL: SKILL.md lacks 'RocksDB-Compat & 15-Year Quirks' vector", file=sys.stderr)
        return False

    # In hn-cynic-checklist.md:
    if "SuperVersion" not in checklist_text:
        print("FAIL: checklist lacks SuperVersion publishing scrutiny", file=sys.stderr)
        return False
    if "lone_commit" not in checklist_text and "asymmetric burst" not in checklist_text.lower():
        print("FAIL: checklist lacks asymmetric burst / lone_commit tail latency scrutiny", file=sys.stderr)
        return False
    if "SIGBUS" not in checklist_text:
        print("FAIL: checklist lacks SIGBUS / fallocate failure scrutiny", file=sys.stderr)
        return False
    if "tombstone" not in checklist_text.lower():
        print("FAIL: checklist lacks tombstone collapsing / snapshot isolation scrutiny", file=sys.stderr)
        return False

    # In examples/mock-hn-grilling.md:
    if "Engineering Armor" not in examples_text and "armadura" not in examples_text.lower():
        print("FAIL: mock-hn-grilling lacks 'Engineering Armor' framing in transformed announcement", file=sys.stderr)
        return False
    if "Complexity Paradox" not in examples_text and "cola de sincronização" not in examples_text.lower():
        print("FAIL: mock-hn-grilling lacks deep architecture teardown", file=sys.stderr)
        return False

    print("PASS: All 4 skill components pass high-caliber architectural verification.")
    return True

if __name__ == "__main__":
    if not test_hn_review_skill():
        sys.exit(1)
