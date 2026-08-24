#!/usr/bin/env python3
"""RFC-0051 P2.2 + P2.3 production guards.

P2.2 — no orphan OS threads in the engine: ``pedradb-core`` production code
must not ``thread::spawn`` / ``thread::Builder::spawn`` (compact/flush
background work goes through Env/Host seams or the PCT turnstile, which
needs every participant to be a registered worker).

P2.3 — logical Clock only: TTL/lease/store code must not read the wall
clock (``SystemTime`` / ``UNIX_EPOCH``); time in the oracle comes from the
logical ``Clock`` (``ManualClock`` in tests).

Heuristic: this codebase keeps tests at the bottom of each file behind a
``#[cfg(test)]`` / ``mod tests`` marker, so a hit counts as a violation
only when it appears BEFORE the file's first such marker (or when the file
has no marker at all).

Usage:
    python3 scripts/check_no_prod_time_spawn.py [repo_root]

Exit 0 = clean; exit 1 = violations (printed).
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

# (directory, violation regex, label)
TARGETS = [
    ("crates/pedradb-core/src", re.compile(r"thread::spawn|thread::Builder::"), "P2.2 spawn"),
    ("crates/pedradb-lease/src", re.compile(r"SystemTime|UNIX_EPOCH"), "P2.3 wall clock"),
    ("crates/pedradb-dcs/src", re.compile(r"SystemTime|UNIX_EPOCH"), "P2.3 wall clock"),
    ("crates/pedradb-store/src", re.compile(r"SystemTime|UNIX_EPOCH"), "P2.3 wall clock"),
    ("crates/pedradb-world/src", re.compile(r"SystemTime|UNIX_EPOCH"), "P2.3 wall clock"),
]

TEST_MARKER = re.compile(r"#\[cfg\(.*test.*\)\]|mod tests")


def check(root: Path) -> int:
    violations = 0
    for rel, pattern, label in TARGETS:
        base = root / rel
        if not base.is_dir():
            print(f"SKIP (missing dir) {rel}")
            continue
        for path in sorted(base.rglob("*.rs")):
            # CLI/bench binaries may stamp report timestamps; the RFC-0051
            # P2.3 rule targets library TTL/lease/oracle logic.
            if "bin" in path.relative_to(base).parts:
                continue
            text = path.read_text(encoding="utf-8", errors="replace").splitlines()
            marker = next(
                (i for i, line in enumerate(text) if TEST_MARKER.search(line)),
                len(text),
            )
            for i, line in enumerate(text[:marker]):
                if pattern.search(line):
                    violations += 1
                    print(f"VIOLATION [{label}] {path.relative_to(root)}:{i + 1}: {line.strip()}")
    return violations


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
    bad = check(root)
    if bad:
        print(f"check_no_prod_time_spawn: {bad} violation(s)")
        return 1
    print("check_no_prod_time_spawn: OK (no production spawns / wall clocks)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
