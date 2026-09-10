#!/usr/bin/env python3
"""RFC-0187 P0.4 — barrier-site floor (blocking, self-verifying).

Every production barrier call site (``sync_data`` / ``sync_all`` /
``sync_dir`` — the fdatasync-class durability barriers) is pinned in
``scripts/ratchet/barrier_sites.tsv`` as an exact per-(file, kind) count.
The gate scans live production source and requires EXACT equality:

- a NEW site not in the floor is red (an uninjected barrier — the P0.3
  crash gate never exercised it);
- a REMOVED site is red (a durability barrier disappeared — that is a
  review event, never a silent pass);
- line-number churn from unrelated edits does NOT red (counts, not lines).

An intentional barrier change moves the TSV in the SAME commit.

``--crash-log FILE`` adds the dynamic tie: parse the P0.3 crash gate's
``CRASH_OPS total=N sync=S`` summary and require S >= 1 (the pinned
barriers actually execute in the crash-injected stream).

``--selftest`` proves redness in both directions without editing any
file (phantom added site; floor entry removed).

``--emit`` rewrites the TSV from the live scan (intentional changes
only; the diff is the review artifact).
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SOURCES = REPO / "crates"
TSV = REPO / "scripts" / "ratchet" / "barrier_sites.tsv"

PATTERN = re.compile(r"\.(sync_data|sync_all|sync_dir)\s*\(")


def scan() -> dict[tuple[str, str], int]:
    """Live per-(file, kind) barrier call-site counts over production
    crates. Tests/benches excluded; comment lines excluded."""
    counts: dict[tuple[str, str], int] = {}
    for rs in sorted(SOURCES.rglob("*.rs")):
        rel = str(rs.relative_to(REPO))
        if "/tests/" in rel or "/benches/" in rel or "/target/" in rel:
            continue
        for line in rs.read_text(encoding="utf-8").splitlines():
            if line.strip().startswith("//"):
                continue
            m = PATTERN.search(line)
            if m:
                key = (rel, m.group(1))
                counts[key] = counts.get(key, 0) + 1
    return counts


def load_floor(text: str) -> dict[tuple[str, str], int]:
    counts: dict[tuple[str, str], int] = {}
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 3:
            raise SystemExit(f"barrier_sites.tsv: bad line (need 3 tab cols): {raw!r}")
        f, kind, n = parts
        counts[(f, kind)] = int(n)
    return counts


def check(floor: dict[tuple[str, str], int], live: dict[tuple[str, str], int]) -> list[str]:
    errs = []
    for key in sorted(set(floor) | set(live)):
        f = floor.get(key, 0)
        l = live.get(key, 0)
        if f == l:
            continue
        direction = "NEW uninjected" if l > f else "REMOVED barrier"
        errs.append(
            f"{key[0]} {key[1]}: live={l} floor={f} ({direction} — update "
            f"scripts/ratchet/barrier_sites.tsv in the SAME commit as the change)"
        )
    return errs


def crash_log_tie(path: Path) -> list[str]:
    """Dynamic tie: the crash gate's summary must show sync-class ops."""
    errs = []
    text = path.read_text(encoding="utf-8")
    m = re.search(r"CRASH_OPS total=(\d+) sync=(\d+)", text)
    if not m:
        return [f"{path}: no CRASH_OPS summary line (P0.3 gate output required)"]
    total, syncs = int(m.group(1)), int(m.group(2))
    if syncs < 1:
        errs.append(f"{path}: sync={syncs} — barrier ops never executed dynamically")
    print(f"BARRIER dynamic tie: crash stream total={total} sync={syncs} (>=1 required)")
    return errs


def selftest() -> int:
    live = scan()
    floor = load_floor(TSV.read_text(encoding="utf-8"))
    base = check(floor, live)
    if base:
        print("SELFTEST barrier: floor already inconsistent with live scan — fix first:")
        for e in base:
            print(f"  {e}")
        return 1

    # Sabotage the LIVE scan (the floor is the pin; the live tree is what
    # a regression changes): an extra site = NEW uninjected; a vanished
    # site = REMOVED barrier. Both must be caught.
    live_plus = dict(live)
    live_plus[("crates/phantom/src/never.rs", "sync_data")] = (
        live_plus.get(("crates/phantom/src/never.rs", "sync_data"), 0) + 1
    )
    added = check(floor, live_plus)
    some_key = max(live.items(), key=lambda kv: kv[1])[0]
    live_minus = dict(live)
    live_minus[some_key] = live_minus[some_key] - 1
    removed = check(floor, live_minus)

    caught = 0
    total = 2
    if any("NEW uninjected" in e for e in added):
        print("SELFTEST barrier: caught=added-uninjected-site")
        caught += 1
    else:
        print("SELFTEST barrier: MISSED added site")
    if any("REMOVED barrier" in e for e in removed):
        print("SELFTEST barrier: caught=removed-barrier-site")
        caught += 1
    else:
        print("SELFTEST barrier: MISSED removed site")
    print(f"SELFTEST barrier: {caught}/{total} sabotages caught")
    return 0 if caught == total else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--selftest", action="store_true", help="prove both red directions")
    ap.add_argument("--emit", action="store_true", help="rewrite TSV from live scan")
    ap.add_argument("--crash-log", type=Path, help="P0.3 gate log for the dynamic tie")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    live = scan()
    if args.emit:
        lines = [
            "# RFC-0187 P0.4 — production barrier-site floor (exact per-(file,kind) counts).",
            "# A NEW sync_data/sync_all/sync_dir site without a same-commit TSV update is a",
            "# red gate (uninjected barrier). A REMOVED site is red too (a durability",
            "# barrier disappeared — review event). Regenerate ONLY via --emit in the same",
            "# commit as an intentional barrier change; the diff is the review artifact.",
        ]
        for (f, kind), n in sorted(live.items()):
            lines.append(f"{f}\t{kind}\t{n}")
        TSV.parent.mkdir(parents=True, exist_ok=True)
        TSV.write_text("\n".join(lines) + "\n", encoding="utf-8")
        print(f"BARRIER floor emitted: {len(live)} (file,kind) entries, {sum(live.values())} sites")
        return 0

    floor = load_floor(TSV.read_text(encoding="utf-8"))
    errs = check(floor, live)
    if args.crash_log:
        errs += crash_log_tie(args.crash_log)
    if errs:
        for e in errs:
            print(f"GATE barrier: FAIL — {e}")
        print("GATE barrier: RED")
        return 1
    print(
        f"GATE barrier: GREEN — {sum(live.values())} production barrier sites pinned "
        f"across {len(live)} (file,kind) entries, exact match"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
