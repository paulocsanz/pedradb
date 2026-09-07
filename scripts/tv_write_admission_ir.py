#!/usr/bin/env python3
"""RFC-0172 P2.1: interpret LLVM IR of write_admission_idle vs the spec.

Restricted interpreter (br/xor/zext/trunc/store/load/ret/icmp). Not a
generic LLVM semantics. The spec is the production term:

    idle(mem, pressure, stall) = !mem && !pressure && !stall
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


def extract_fn(ll: str, name: str) -> str:
    # rustc emits `; crate::name` then optional `Function Attrs` then `define`.
    # Require exact suffix so `write_admission_idle` ≠ `write_admission_idle_as_is`.
    needle = f"; write_admission_kernel::{name}\n"
    start = ll.find(needle)
    if start < 0:
        raise SystemExit(f"FAIL  IR missing function {name}")
    d = ll.find("\ndefine ", start)
    if d < 0:
        raise SystemExit(f"FAIL  IR define missing for {name}")
    rest = ll[d + 1 :]
    depth = 0
    for i, c in enumerate(rest):
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return rest[: i + 1]
    raise SystemExit(f"FAIL  unbalanced IR for {name}")


def _blocks(body: str) -> dict[str, list[str]]:
    """Split `define ... { start: ... bb1: ... }` into label -> insts."""
    inner = body[body.find("{") + 1 : body.rfind("}")]
    blocks: dict[str, list[str]] = {}
    cur = "entry"
    blocks[cur] = []
    for raw in inner.splitlines():
        line = raw.split(";")[0].strip()
        if not line:
            continue
        if line.endswith(":") and not line.startswith("define"):
            cur = line[:-1].strip()
            blocks.setdefault(cur, [])
            continue
        blocks[cur].append(line)
    return blocks


def interpret_idle(fn: str, mem: bool, pressure: bool, stall: bool) -> bool:
    regs: dict[str, object] = {
        "%mem_stall": mem,
        "%pressure_l0": pressure,
        "%stall_l0": stall,
    }
    mem8: dict[str, int] = {}
    blocks = _blocks(fn)
    # first real block is `start:`
    pc_label = "start" if "start" in blocks else next(iter(blocks))
    for _ in range(64):
        insts = blocks[pc_label]
        i = 0
        while i < len(insts):
            line = insts[i]
            if line.startswith("br i1 "):
                # br i1 %x, label %a, label %b
                m = re.match(
                    r"br i1 (%[\w.]+), label (%[\w.]+), label (%[\w.]+)",
                    line,
                )
                if not m:
                    raise SystemExit(f"FAIL  unhandled br: {line}")
                cond = bool(regs[m.group(1)])
                pc_label = m.group(2)[1:] if cond else m.group(3)[1:]
                break
            if line.startswith("br label "):
                m = re.match(r"br label (%[\w.]+)", line)
                if not m:
                    raise SystemExit(f"FAIL  unhandled br label: {line}")
                pc_label = m.group(1)[1:]
                break
            if line.startswith("ret i1 "):
                v = line.split("ret i1 ", 1)[1].strip()
                if v in ("true", "false"):
                    return v == "true"
                return bool(regs[v])
            if " = xor i1 " in line:
                dest, rest = line.split("=", 1)
                m = re.match(r"\s*xor i1 (%[\w.]+), (true|false)", rest)
                if not m:
                    raise SystemExit(f"FAIL  unhandled xor: {line}")
                a = bool(regs[m.group(1)])
                b = m.group(2) == "true"
                regs[dest.strip()] = a ^ b
                i += 1
                continue
            if " = zext i1 " in line:
                dest, rest = line.split("=", 1)
                m = re.match(r"\s*zext i1 (%[\w.]+) to i8", rest)
                if not m:
                    raise SystemExit(f"FAIL  unhandled zext: {line}")
                regs[dest.strip()] = 1 if regs[m.group(1)] else 0
                i += 1
                continue
            if " = trunc" in line and " to i1" in line:
                dest, rest = line.split("=", 1)
                m = re.search(r"trunc(?: \w+)? i8 (%[\w.]+) to i1", rest)
                if not m:
                    raise SystemExit(f"FAIL  unhandled trunc: {line}")
                regs[dest.strip()] = bool(int(regs[m.group(1)]))
                i += 1
                continue
            if line.startswith("store i8 "):
                m = re.match(r"store i8 (\d+|%[\w.]+), ptr (%[\w.]+)", line)
                if not m:
                    raise SystemExit(f"FAIL  unhandled store: {line}")
                val = m.group(1)
                ptr = m.group(2)
                mem8[ptr] = int(regs[val]) if val.startswith("%") else int(val)
                i += 1
                continue
            if " = load i8, ptr " in line:
                dest, rest = line.split("=", 1)
                m = re.search(r"ptr (%[\w.]+)", rest)
                if not m:
                    raise SystemExit(f"FAIL  unhandled load: {line}")
                regs[dest.strip()] = mem8[m.group(1)]
                i += 1
                continue
            if line.startswith("%") and " = alloca " in line:
                i += 1
                continue
            raise SystemExit(f"FAIL  unhandled inst: {line}")
        else:
            raise SystemExit(f"FAIL  block {pc_label} fell off")
    raise SystemExit("FAIL  interpreter loop")


def spec(mem: bool, pressure: bool, stall: bool) -> bool:
    return (not mem) and (not pressure) and (not stall)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: tv_write_admission_ir.py FILE.ll", file=sys.stderr)
        return 2
    ll = Path(sys.argv[1]).read_text(encoding="utf-8")
    fn = extract_fn(ll, "write_admission_idle")
    bad = []
    for mem in (False, True):
        for pressure in (False, True):
            for stall in (False, True):
                got = interpret_idle(fn, mem, pressure, stall)
                want = spec(mem, pressure, stall)
                if got != want:
                    bad.append((mem, pressure, stall, got, want))
    if bad:
        print("FAIL  IR ⊭ idle spec:", bad[:4])
        return 1
    print("ok    TV write_admission_idle 8/8 IR matches spec")
    return 0


if __name__ == "__main__":
    sys.exit(main())
