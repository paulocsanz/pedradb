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
            if " = icmp " in line:
                dest, rest = line.split("=", 1)
                m = re.match(
                    r"\s*icmp (eq|ne|ugt|uge|ult|ule|sgt|sge|slt|sle) "
                    r"(i\d+) (%[\w.]+|\d+), (%[\w.]+|\d+)",
                    rest,
                )
                if not m:
                    raise SystemExit(f"FAIL  unhandled icmp: {line}")
                pred, _ty, a, b = m.group(1), m.group(2), m.group(3), m.group(4)

                def _ival(tok: str) -> int:
                    if tok.startswith("%"):
                        v = regs[tok]
                        return int(v) if not isinstance(v, bool) else int(v)
                    return int(tok)

                av, bv = _ival(a), _ival(b)
                if pred == "eq":
                    regs[dest.strip()] = av == bv
                elif pred == "ne":
                    regs[dest.strip()] = av != bv
                elif pred == "ugt":
                    regs[dest.strip()] = av > bv
                elif pred == "uge":
                    regs[dest.strip()] = av >= bv
                elif pred == "ult":
                    regs[dest.strip()] = av < bv
                elif pred == "ule":
                    regs[dest.strip()] = av <= bv
                elif pred == "sgt":
                    regs[dest.strip()] = av > bv
                elif pred == "sge":
                    regs[dest.strip()] = av >= bv
                elif pred == "slt":
                    regs[dest.strip()] = av < bv
                elif pred == "sle":
                    regs[dest.strip()] = av <= bv
                else:
                    raise SystemExit(f"FAIL  unhandled icmp pred: {line}")
                i += 1
                continue
            if line.startswith("ret i8 "):
                v = line.split("ret i8 ", 1)[1].strip()
                if v.isdigit():
                    return int(v)
                return int(regs[v])
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


def interpret_admit(
    fn: str,
    mem_bytes: int,
    mem_armed: bool,
    mem_limit: int,
    l0: int,
    l0_armed: bool,
    l0_limit: int,
) -> int:
    regs: dict[str, object] = {
        "%mem_bytes": mem_bytes,
        "%mem_armed": mem_armed,
        "%mem_limit": mem_limit,
        "%l0": l0,
        "%l0_armed": l0_armed,
        "%l0_limit": l0_limit,
    }
    mem8: dict[str, int] = {}
    blocks = _blocks(fn)
    pc_label = "start" if "start" in blocks else next(iter(blocks))
    for _ in range(64):
        insts = blocks[pc_label]
        i = 0
        while i < len(insts):
            line = insts[i]
            if line.startswith("br i1 "):
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
            if line.startswith("ret i8 "):
                v = line.split("ret i8 ", 1)[1].strip()
                return int(v) if v.isdigit() else int(regs[v])
            if " = icmp " in line:
                dest, rest = line.split("=", 1)
                m = re.match(
                    r"\s*icmp (eq|ne|ugt|uge|ult|ule) i\d+ (%[\w.]+|\d+), (%[\w.]+|\d+)",
                    rest,
                )
                if not m:
                    raise SystemExit(f"FAIL  unhandled icmp: {line}")

                def _ival(tok: str) -> int:
                    if tok.startswith("%"):
                        v = regs[tok]
                        return int(v) if not isinstance(v, bool) else int(v)
                    return int(tok)

                av, bv = _ival(m.group(2)), _ival(m.group(3))
                pred = m.group(1)
                regs[dest.strip()] = {
                    "eq": av == bv,
                    "ne": av != bv,
                    "ugt": av > bv,
                    "uge": av >= bv,
                    "ult": av < bv,
                    "ule": av <= bv,
                }[pred]
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


# WriteAdmit: Ok=0, StallMem=1, StallL0=2 (rustc enum layout at opt-level=0).
def spec_admit(
    mem_bytes: int,
    mem_armed: bool,
    mem_limit: int,
    l0: int,
    l0_armed: bool,
    l0_limit: int,
) -> int:
    if mem_armed and mem_bytes >= mem_limit:
        return 1
    if l0_armed and l0 >= l0_limit:
        return 2
    return 0


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

    admit = extract_fn(ll, "write_admit")
    bad_a = []
    mem_limit, l0_limit = 50, 4
    for mem_armed in (False, True):
        for l0_armed in (False, True):
            for mem_bytes in (0, 50, 100):
                for l0 in (0, 4, 8):
                    got = interpret_admit(
                        admit, mem_bytes, mem_armed, mem_limit, l0, l0_armed, l0_limit
                    )
                    want = spec_admit(
                        mem_bytes, mem_armed, mem_limit, l0, l0_armed, l0_limit
                    )
                    if got != want:
                        bad_a.append(
                            (mem_bytes, mem_armed, l0, l0_armed, got, want)
                        )
    if bad_a:
        print("FAIL  IR ⊭ write_admit spec:", bad_a[:4])
        return 1
    n = 2 * 2 * 3 * 3
    print(f"ok    TV write_admit {n}/{n} IR matches spec")
    return 0


if __name__ == "__main__":
    sys.exit(main())
