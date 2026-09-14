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

# RFC-0222 P0.3: a file whose module is declared under a #[cfg(...test...)]
# attribute in its parent (lib.rs/main.rs/mod.rs of the same directory) is
# test-only even when the file itself carries no in-file marker — the
# heuristic below used to false-positive on such harnesses (98 hits in
# pedradb-store's three_teeth_queued.rs, cfg(test)-gated in lib.rs).
TEST_ATTR = re.compile(r"#\[cfg\([^)]*test[^)]*\)\]")


def _test_gated_modules(parent: Path) -> set[str]:
    """Module names declared as `mod name;` directly under a cfg(test) attr."""
    gated: set[str] = set()
    if not parent.is_file():
        return gated
    lines = parent.read_text(encoding="utf-8", errors="replace").splitlines()
    for i, line in enumerate(lines):
        m = re.match(r"\s*mod (\w+)\s*;", line)
        if not m or i == 0:
            continue
        window = [ln.strip() for ln in lines[max(0, i - 3) : i]]
        if any(TEST_ATTR.match(ln) for ln in window):
            gated.add(m.group(1))
    return gated


def _module_test_gated(path: Path) -> bool:
    """True when the file's `mod` declaration is cfg(test)-gated upstream."""
    stems = {path.stem}
    if path.stem.endswith("_kernel"):
        stems.add(path.stem[: -len("_kernel")])
    for parent_name in (
        "lib.rs",
        "lib_kernel.rs",
        "main.rs",
        "main_kernel.rs",
        "mod.rs",
        "mod_kernel.rs",
    ):
        gated = _test_gated_modules(path.parent / parent_name)
        if stems & gated:
            return True
    return False



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
            if _module_test_gated(path):
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


def selftest() -> int:
    """Sabotage proof (RFC-0222 P0.3): a cfg(test)-gated harness module is
    exempt, a real production hit still fires — both in one synthetic tree."""
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        src = root / "crates/pedradb-store/src"
        src.mkdir(parents=True)
        (src / "lib.rs").write_text(
            "#[cfg(test)]\nmod harness;\n\nmod prod;\n", encoding="utf-8"
        )
        (src / "harness.rs").write_text(
            "fn t() { let _ = std::time::SystemTime::now(); }\n", encoding="utf-8"
        )
        (src / "prod.rs").write_text(
            "fn p() { let _ = std::time::SystemTime::now(); }\n", encoding="utf-8"
        )
        n = check(root)
        if n != 1:
            print(f"selftest: FAIL — expected exactly 1 violation (prod.rs), got {n}")
            return 1
        print("selftest: OK — gated harness exempt, production hit fires")
        return 0


def main() -> int:
    argv = sys.argv[1:]
    if "--selftest" in argv:
        return selftest()
    root = Path(argv[0]) if argv else Path(__file__).resolve().parent.parent
    bad = check(root)
    if bad:
        print(f"check_no_prod_time_spawn: {bad} violation(s)")
        return 1
    print("check_no_prod_time_spawn: OK (no production spawns / wall clocks)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
