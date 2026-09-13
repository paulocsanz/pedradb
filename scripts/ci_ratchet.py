#!/usr/bin/env python3
"""CI ratchets: fail on regressions, tolerate the registered debt.

Usage:
  ci_ratchet.py tests <cargo-test-log> <baseline-file>
  ci_ratchet.py lint <formal-lint-log> <max-fail-count>

`tests`: the set of failing tests parsed from a `cargo test` log must be a
subset of the baseline file (one test name per line). A new failure exits 1;
a shrinking set prints the names to drop from the baseline and exits 0.

`lint`: FAIL lines from `pedra_formal.py --lint`. Hard guarantee: no FAIL
may name drift (extract stamps, twins, clones) — any drift FAIL exits 1.
The remaining FAILs are registered classification debt (residuals freeze,
tcb freeze, in-flight rows mirrored from the main tree); their count must
not exceed max-fail-count.
"""

import re
import sys

FAIL_HEADER = re.compile(r"^---- (\S+) stdout ----$", re.MULTILINE)


def cmd_tests(log_path: str, baseline_path: str) -> int:
    log = open(log_path, encoding="utf-8", errors="replace").read()
    actual = sorted(set(FAIL_HEADER.findall(log)))
    baseline = {
        line.strip()
        for line in open(baseline_path, encoding="utf-8")
        if line.strip()
    }
    new = [name for name in actual if name not in baseline]
    if new:
        print(f"RATCHET FAIL: {len(new)} new failing test(s):")
        for name in new:
            print(f"  {name}")
        return 1
    fixed = sorted(baseline - set(actual))
    if fixed:
        print(f"ratchet: {len(fixed)} known failure(s) now pass; drop them from {baseline_path}:")
        for name in fixed:
            print(f"  {name}")
    print(f"ratchet: {len(actual)} failing, all {len(baseline)} known")
    return 0


def cmd_lint(log_path: str, max_fail: int) -> int:
    fails = [
        line
        for line in open(log_path, encoding="utf-8", errors="replace")
        if line.startswith("FAIL")
    ]
    drift = [line for line in fails if "drift" in line.lower()]
    if drift:
        print("RATCHET FAIL: drift FAILs are never tolerated:")
        for line in drift:
            print(f"  {line.rstrip()}")
        return 1
    if len(fails) > max_fail:
        print(
            f"RATCHET FAIL: {len(fails)} lint FAILs exceeds the registered "
            f"debt ceiling of {max_fail}"
        )
        return 1
    print(f"ratchet: {len(fails)} lint FAILs, within the {max_fail} debt ceiling")
    return 0


def main() -> int:
    if len(sys.argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    mode, log_path, arg = sys.argv[1], sys.argv[2], sys.argv[3]
    if mode == "tests":
        return cmd_tests(log_path, arg)
    if mode == "lint":
        return cmd_lint(log_path, int(arg))
    print(f"unknown mode: {mode}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
