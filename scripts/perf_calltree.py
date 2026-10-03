#!/usr/bin/env python3
"""perf_calltree.py — parse macOS `sample` call graphs into a bottleneck map.

Turns a `sample <pid> <secs> -file x.txt` dump (or several) into:
  * per-thread totals (who is busy vs sleeping),
  * top frames by SELF time (the leaves where cycles actually go),
  * subtree rollup under a symbol filter (e.g. the hydrate path),
  * blocking map (locks / sleeps / yields / futex-ish frames),
  * diff mode between two profiles (same tree, two runs).

Usage:
  perf_calltree.py sample.txt [--top 15] [--under hydrate] [--threads]
  perf_calltree.py --diff before.txt after.txt [--under write_sst]
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field


@dataclass
class Frame:
    count: int
    name: str
    children: list["Frame"] = field(default_factory=list)

    @property
    def self_time(self) -> int:
        return self.count - sum(c.count for c in self.children)

    def walk(self, depth=0):
        yield self, depth
        for c in self.children:
            yield from c.walk(depth + 1)


def parse_sample(path: str) -> dict[str, Frame]:
    """Parse a `sample` text dump into {thread_header: root_frame}."""
    threads: dict[str, Frame] = {}
    cur_thread: str | None = None
    # Stack of (indent, frame) — the tree chars (+ ! : |) each add 2 columns.
    stack: list[tuple[int, Frame]] = []
    with open(path, errors="replace") as fh:
        for line in fh:
            m = re.match(r"\s*(\d+)\s+(Thread_\d+.*)", line)
            if m:
                cur_thread = m.group(2).strip()
                root = Frame(int(m.group(1)), cur_thread)
                threads[cur_thread] = root
                stack = [(0, root)]
                continue
            if cur_thread is None:
                continue
            lm = re.match(r"(\s*)\d+\s+(.*)", line)
            if not lm:
                # Section breaks ("Call graph:", blank) end the current thread.
                if line.strip() in ("", "Call graph:"):
                    continue
                if not re.match(r"\s*[+!|:\-\s]", line) and stack:
                    cur_thread = None
                    stack = []
                continue
            indent = len(lm.group(1))
            cm = re.match(r"\s*(\d+)", line)
            count = int(cm.group(1))
            name = lm.group(2).strip()
            if not stack:
                continue
            while stack and indent <= stack[-1][0]:
                stack.pop()
            frame = Frame(count, name)
            stack[-1][1].children.append(frame)
            stack.append((indent, frame))
    return threads


def top_self(threads: dict[str, Frame], n: int, filt: str | None):
    rows: list[tuple[int, str]] = []
    for hdr, root in threads.items():
        for f, depth in root.walk():
            if f.self_time <= 0:
                continue
            if filt and filt not in f.name:
                continue
            rows.append((f.self_time, f.name[:120]))
    rows.sort(reverse=True)
    total = sum(r.count for r in [root for root in threads.values()])
    for c, name in rows[:n]:
        print(f"{c:>8}  {name}")
    print(f"(thread-total samples: {sum(t.count for t in threads.values())})")


def subtree(threads: dict[str, Frame], needle: str, n: int):
    for hdr, root in threads.items():
        for f, depth in root.walk():
            if needle in f.name and depth <= 12:
                print(f"\n== subtree under [{f.name[:100]}] count={f.count} ({hdr[:60]}) ==")
                kids = sorted(f.children, key=lambda c: -c.count)[:n]
                for c in kids:
                    print(f"  {c.count:>7}  {c.name[:110]}")
                return  # first match is enough


BLOCK_PAT = re.compile(
    r"lock_exclusive_slow|lock_shared_slow|cthread_yield|__psynch_cvwait|"
    r"nanosleep|__semwait|recv_timeout|kqueue|select\$|__ulock|swtch_pri|"
    r"mach_msg2_trap|poll\)"
)


def blocking_map(threads: dict[str, Frame], n: int):
    rows: dict[str, int] = {}
    for hdr, root in threads.items():
        for f, _ in root.walk():
            if BLOCK_PAT.search(f.name):
                key = f.name.split(" ")[0][:100]
                rows[key] = rows.get(key, 0) + f.count
    print("== blocking frames (accumulated) ==")
    for k, v in sorted(rows.items(), key=lambda kv: -kv[1])[:n]:
        print(f"{v:>8}  {k}")


def thread_totals(threads: dict[str, Frame]):
    print("== thread totals ==")
    for hdr, root in sorted(threads.items(), key=lambda kv: kv[1].count, reverse=True)[:16]:
        busy = root.count - sum(
            f.count for f, _ in root.walk()
            if BLOCK_PAT.search(f.name)
        )
        print(f"{root.count:>8} (busy≈{busy:>7})  {hdr[:80]}")


def diff(a: dict[str, Frame], b: dict[str, Frame], needle: str | None):
    def collect(threads):
        acc: dict[str, int] = {}
        for root in threads.values():
            for f, _ in root.walk():
                acc[f.name] = acc.get(f.name, 0) + f.count
        return acc

    ca, cb = collect(a), collect(b)
    keys = sorted(set(ca) | set(cb), key=lambda k: abs(cb.get(k, 0) - ca.get(k, 0)), reverse=True)
    print(f"{'Δ':>8} {'before':>8} {'after':>8}  frame")
    shown = 0
    for k in keys:
        if needle and needle not in k:
            continue
        d = cb.get(k, 0) - ca.get(k, 0)
        if abs(d) < max(20, 0.01 * max(ca.get(k, 0), cb.get(k, 0))):
            continue
        print(f"{d:>+8} {ca.get(k, 0):>8} {cb.get(k, 0):>8}  {k[:110]}")
        shown += 1
        if shown >= 25:
            break


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("paths", nargs="+")
    ap.add_argument("--top", type=int, default=15)
    ap.add_argument("--under")
    ap.add_argument("--threads", action="store_true")
    ap.add_argument("--blocking", action="store_true")
    ap.add_argument("--diff", action="store_true")
    args = ap.parse_args()

    parsed = [parse_sample(p) for p in args.paths]
    if args.diff and len(parsed) >= 2:
        diff(parsed[0], parsed[1], args.under)
        return
    t = parsed[0]
    if args.threads:
        thread_totals(t)
    if args.blocking:
        blocking_map(t, args.top)
    if args.under:
        subtree(t, args.under, args.top)
    else:
        top_self(t, args.top, None)


if __name__ == "__main__":
    sys.exit(main())
