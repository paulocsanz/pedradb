#!/usr/bin/env python3
"""Pedra formal glue: caller lint, twin-token diff, clone drift, models, Verus.

The production kernel is the source of truth. A Verus file is a twin, not a
second implementation. This script does not prove anything — it refuses
silent drift between those copies, refuses a close twin that lacks the
entry fn, and refuses a kernel nobody in production calls. See
scripts/formal/README.md.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

FN_HEAD = re.compile(
    r"(?P<pre>(?:pub\s+)?(?:open\s+)?(?:spec\s+)?(?:proof\s+)?)fn\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*[<(]",
)
TOKEN_RE = re.compile(
    r"0x[0-9a-fA-F]+|\b\d+u(?:8|16|32|64)?\b|\b\d+\b|\btrue\b|\bfalse\b|"
    r"==|!=|<=|>=|&&|\|\||<(?!<)|>(?!>)|"
    r"::[A-Z][A-Za-z0-9_]*"
)
SKIP_FN = re.compile(r"(_as_is|_spec)$")


def strip_comments(src: str) -> str:
    """Drop // and /* */ comments without touching // inside strings."""
    out: list[str] = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if c in "\"'":
            quote = c
            out.append(c)
            i += 1
            while i < n:
                out.append(src[i])
                if src[i] == "\\" and i + 1 < n:
                    out.append(src[i + 1])
                    i += 2
                    continue
                if src[i] == quote:
                    i += 1
                    break
                i += 1
            continue
        if c == "/" and nxt == "/":
            i += 2
            while i < n and src[i] != "\n":
                i += 1
            continue
        if c == "/" and nxt == "*":
            i += 2
            while i + 1 < n and not (src[i] == "*" and src[i + 1] == "/"):
                i += 1
            i = min(n, i + 2)
            continue
        out.append(c)
        i += 1
    return "".join(out)


def match_braces(src: str, open_at: int) -> int:
    depth = 0
    i = open_at
    n = len(src)
    while i < n:
        c = src[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        elif c == '"':
            i += 1
            while i < n and src[i] != '"':
                if src[i] == "\\":
                    i += 1
                i += 1
        i += 1
    raise ValueError("unbalanced braces")


def iter_fns(src: str):
    text = strip_comments(src)
    for m in FN_HEAD.finditer(text):
        pre = m.group("pre") or ""
        name = m.group("name")
        kind = "exec"
        if "spec" in pre.split():
            kind = "spec"
        elif "proof" in pre.split():
            kind = "proof"
        brace = text.find("{", m.end() - 1)
        if brace < 0:
            continue
        end = match_braces(text, brace)
        yield kind, name, text[brace + 1 : end]


def exec_fns(src: str) -> dict[str, str]:
    out = {}
    for kind, name, body in iter_fns(src):
        if kind != "exec" or SKIP_FN.search(name):
            continue
        out[name] = body
    return out


def spec_fns(src: str) -> dict[str, str]:
    out = {}
    for kind, name, body in iter_fns(src):
        if kind != "spec":
            continue
        out[name] = body
    return out


def tokens(body: str) -> list[str]:
    out = []
    for t in TOKEN_RE.findall(body):
        m = re.match(r"(\d+)u(?:8|16|32|64)?$", t)
        out.append(m.group(1) if m else t)
    return out


def load_text(root: Path, rel: str) -> str | None:
    p = root / rel
    if not p.is_file():
        return None
    return p.read_text(encoding="utf-8")


def mentions(src: str, symbol: str) -> bool:
    return (
        re.search(r"\b" + re.escape(symbol) + r"\s*\(", strip_comments(src))
        is not None
    )


def find_verus(root: Path) -> str | None:
    env = os.environ.get("VERUS")
    if env and os.path.isfile(env) and os.access(env, os.X_OK):
        return env
    home = Path.home() / ".local/verus/verus-arm64-macos/verus"
    if home.is_file() and os.access(home, os.X_OK):
        return str(home)
    which = subprocess.run(
        "command -v verus", shell=True, capture_output=True, text=True
    )
    path = which.stdout.strip()
    return path or None


class Report:
    def __init__(self) -> None:
        self.failed: list[str] = []
        self.gaps: list[str] = []
        self.ok: list[str] = []

    def fail(self, msg: str) -> None:
        self.failed.append(msg)
        print(f"FAIL  {msg}")

    def gap(self, msg: str) -> None:
        self.gaps.append(msg)
        print(f"GAP   {msg}")

    def good(self, msg: str) -> None:
        self.ok.append(msg)
        print(f"ok    {msg}")


def check_lint(root: Path, catalog: dict, r: Report) -> None:
    print("== lint (production must call the kernel) ==")
    for pair in catalog["pairs"]:
        entry = pair.get("entry")
        if not entry:
            continue
        kernel = pair["kernel"]
        for caller in pair.get("callers", []):
            src = load_text(root, caller)
            if src is None:
                r.fail(f"{pair['id']}: missing caller {caller}")
                continue
            if not mentions(src, entry):
                r.fail(f"{pair['id']}: {caller} does not call {entry}()")
            elif caller == kernel:
                r.good(f"{pair['id']}: {entry} lives and is used in {caller}")
            else:
                r.good(f"{pair['id']}: {caller} calls {entry}")


def check_clones(root: Path, catalog: dict, r: Report) -> None:
    print("== clones (duplicated production kernels) ==")
    for clone in catalog.get("clones", []):
        a = load_text(root, clone["a"])
        b = load_text(root, clone["b"])
        if a is None or b is None:
            r.fail(f"{clone['id']}: missing {clone['a'] if a is None else clone['b']}")
            continue
        af, bf = exec_fns(a), exec_fns(b)
        for name in clone["fns"]:
            if name not in af or name not in bf:
                r.fail(f"{clone['id']}: {name} missing in one side")
                continue
            ta, tb = tokens(af[name]), tokens(bf[name])
            if ta != tb:
                r.fail(
                    f"{clone['id']}: {name} drifted ({ta} vs {tb})"
                )
            else:
                r.good(f"{clone['id']}: {name} identical tokens")


TWIN_KINDS = {"close", "atom", "model"}


def check_twins(root: Path, catalog: dict, r: Report, strict: bool) -> None:
    print("== twins (kind + kernel tokens ⊆ Verus twin) ==")
    for pair in catalog["pairs"]:
        pid = pair["id"]
        kind = pair.get("twin_kind")
        if kind not in TWIN_KINDS:
            r.fail(f"{pid}: twin_kind must be close|atom|model (got {kind!r})")
            continue
        absent = pair.get("status") == "absent"
        ksrc = load_text(root, pair["kernel"])
        if ksrc is None:
            r.fail(f"{pid}: missing kernel {pair['kernel']}")
            continue
        tsrc = load_text(root, pair["twin"])
        if tsrc is None:
            msg = f"{pid}: missing twin {pair['twin']}"
            if absent and not strict:
                r.gap(msg)
            else:
                r.fail(msg)
            continue
        if absent:
            r.good(f"{pid}: twin present (catalog still says absent — update catalog)")
        kexec = exec_fns(ksrc)
        texec = exec_fns(tsrc)
        # Verus `ensures` clauses contain `{`; compare against the whole twin
        # (comments already stripped) so we do not have to parse proof syntax.
        twin_tok = set(tokens(strip_comments(tsrc)))

        if kind == "close":
            name = pair.get("token_src") or pair.get("entry")
            if not name:
                r.fail(f"{pid}: close twin needs entry or token_src")
                continue
            if name not in texec:
                r.fail(
                    f"{pid}: close twin missing exec fn {name}() "
                    f"(got {sorted(texec)}; mark twin_kind=atom if this is a cartoon)"
                )
                continue
            names = [name]
        elif kind == "atom":
            atom = pair.get("atom")
            if not atom:
                r.fail(f"{pid}: atom twin needs atom= (the fn the Verus file proves)")
                continue
            if atom not in texec:
                r.fail(f"{pid}: atom twin missing exec fn {atom}() (got {sorted(texec)})")
                continue
            if atom in kexec:
                names = [atom]
            elif pair.get("token_src") in kexec:
                names = [pair["token_src"]]
            else:
                r.good(f"{pid}: atom {atom} (not in kernel; twin-only)")
                continue
        else:  # model
            name = pair.get("token_src") or pair.get("entry")
            if not name:
                r.fail(f"{pid}: model twin needs entry or token_src")
                continue
            if name not in texec:
                r.fail(f"{pid}: model twin missing exec fn {name}()")
                continue
            names = [name]

        for name in names:
            if name not in kexec:
                r.fail(f"{pid}: {name} not in kernel")
                continue
            missing = [t for t in tokens(kexec[name]) if t not in twin_tok]
            if missing:
                r.fail(f"{pid}: {name} twin missing tokens {missing}")
            elif kind == "model":
                r.good(f"{pid}: {name} tokens covered (model: stand-in domain, not production types)")
            elif kind == "atom":
                r.good(f"{pid}: atom {name} tokens covered (not the production entry)")
            else:
                r.good(f"{pid}: close {name} tokens covered")


def check_scripts(root: Path, catalog: dict, r: Report, strict: bool) -> None:
    print("== verus scripts (SRC must match catalog) ==")
    catalog_scripts = {}
    for pair in catalog["pairs"]:
        script = pair.get("verus")
        if script:
            catalog_scripts[script] = pair
    for sh in sorted((root / "scripts").glob("verus_*.sh")):
        rel = f"scripts/{sh.name}"
        if rel not in catalog_scripts:
            r.fail(f"unlisted {rel}")
            continue
        pair = catalog_scripts[rel]
        text = sh.read_text(encoding="utf-8")
        m = re.search(r'SRC="\$ROOT/([^"]+)"', text)
        if not m:
            r.fail(f"{rel}: no SRC= line")
            continue
        src = m.group(1)
        if src != pair["twin"]:
            r.fail(f"{rel}: SRC {src} != catalog twin {pair['twin']}")
            continue
        exists = (root / src).is_file()
        if not exists:
            msg = f"{rel}: SRC missing on disk ({src})"
            if pair.get("status") == "absent" and not strict:
                r.gap(msg)
            else:
                r.fail(msg)
        else:
            r.good(f"{rel} → {src}")


def check_models(root: Path, catalog: dict, r: Report) -> None:
    print("== models (Stateright on production kernels) ==")
    env = os.environ.copy()
    env.setdefault("CARGO_TERM_COLOR", "never")
    for model in catalog.get("models", []):
        cmd = [
            "cargo",
            "test",
            "-q",
            "-p",
            model["crate"],
            "--test",
            model["test"],
            "--",
            "--test-threads=1",
        ]
        print("      " + " ".join(cmd))
        p = subprocess.run(cmd, cwd=root, env=env)
        if p.returncode != 0:
            r.fail(f"model {model['crate']}::{model['test']} exit {p.returncode}")
        else:
            r.good(f"model {model['crate']}::{model['test']}")


def check_extract(
    root: Path, r: Report, *, want_charon: bool, charon_required: bool
) -> None:
    print("== extract (include crate; Charon/Aeneas optional) ==")
    manifest = root / "formal/aeneas/vote-kernel/Cargo.toml"
    if not manifest.is_file():
        r.fail("formal/aeneas/vote-kernel/Cargo.toml missing")
        return
    env = os.environ.copy()
    env.setdefault("CARGO_TERM_COLOR", "never")
    p = subprocess.run(
        [
            "cargo",
            "test",
            "-q",
            "--manifest-path",
            str(manifest),
            "--",
            "--test-threads=1",
        ],
        cwd=root,
        env=env,
    )
    if p.returncode != 0:
        r.fail(f"aeneas include crate cargo test exit {p.returncode}")
        return
    r.good("aeneas extract crate (production vote_kernel.rs as [lib] path)")
    lean = root / "formal/aeneas/out/lean/VoteKernel.lean"
    if lean.is_file() and "def vote_decision" in lean.read_text(encoding="utf-8"):
        r.good("aeneas extract artifact has def vote_decision")
    else:
        r.gap("aeneas extract artifact formal/aeneas/out/lean/VoteKernel.lean missing (run ./scripts/aeneas_vote.sh)")
    stamp = root / "formal/aeneas/out/SOURCE"
    src = root / "crates/pedradb-raft/src/vote_kernel.rs"
    if stamp.is_file() and src.is_file():
        want = None
        for line in stamp.read_text(encoding="utf-8").splitlines():
            if line.startswith("sha256="):
                want = line.split("=", 1)[1].strip()
        have = hashlib.sha256(src.read_bytes()).hexdigest()
        if want and have == want:
            r.good("aeneas SOURCE sha256 matches vote_kernel.rs")
        elif want:
            r.fail(
                f"aeneas SOURCE sha256 drifted (kernel {have[:12]}… vs stamp {want[:12]}…; re-run ./scripts/aeneas_vote.sh)"
            )
    iso_lean = root / "formal/aeneas/out/lean/IsolatedKernel.lean"
    if iso_lean.is_file() and "def isolated_id_matches" in iso_lean.read_text(
        encoding="utf-8"
    ):
        r.good("aeneas extract artifact has def isolated_id_matches")
    else:
        r.gap(
            "aeneas extract artifact IsolatedKernel.lean missing (run ./scripts/aeneas_isolated.sh)"
        )
    iso_stamp = root / "formal/aeneas/out/SOURCE.isolated"
    iso_src = root / "crates/pedradb-fold/src/isolated_kernel.rs"
    if iso_stamp.is_file() and iso_src.is_file():
        want = None
        for line in iso_stamp.read_text(encoding="utf-8").splitlines():
            if line.startswith("sha256="):
                want = line.split("=", 1)[1].strip()
        have = hashlib.sha256(iso_src.read_bytes()).hexdigest()
        if want and have == want:
            r.good("aeneas SOURCE.isolated sha256 matches isolated_kernel.rs")
        elif want:
            r.fail(
                f"aeneas SOURCE.isolated drifted (kernel {have[:12]}… vs stamp {want[:12]}…; re-run ./scripts/aeneas_isolated.sh)"
            )
    bloom_lean = root / "formal/aeneas/out/lean/BloomKernel.lean"
    if bloom_lean.is_file() and "def BloomFilter.may_contain" in bloom_lean.read_text(
        encoding="utf-8"
    ):
        r.good("aeneas extract artifact has def BloomFilter.may_contain")
    else:
        r.gap(
            "aeneas extract artifact BloomKernel.lean missing (run ./scripts/aeneas_bloom.sh)"
        )
    bloom_stamp = root / "formal/aeneas/out/SOURCE.bloom"
    bloom_src = root / "crates/pedradb-core/src/bloom.rs"
    if bloom_stamp.is_file() and bloom_src.is_file():
        want = None
        for line in bloom_stamp.read_text(encoding="utf-8").splitlines():
            if line.startswith("sha256="):
                want = line.split("=", 1)[1].strip()
        have = hashlib.sha256(bloom_src.read_bytes()).hexdigest()
        if want and have == want:
            r.good("aeneas SOURCE.bloom sha256 matches bloom.rs")
        elif want:
            r.fail(
                f"aeneas SOURCE.bloom drifted (kernel {have[:12]}… vs stamp {want[:12]}…; re-run ./scripts/aeneas_bloom.sh)"
            )
    lean_script = root / "scripts/lean_vote.sh"
    p = subprocess.run(
        ["bash", str(lean_script)] + (["--required"] if charon_required else []),
        cwd=root,
    )
    if p.returncode != 0:
        r.fail(f"lean_vote.sh exit {p.returncode}")
    elif charon_required:
        r.good("lean_vote.sh (vote_decision_matches_spec)")
    if not (want_charon or charon_required):
        return
    script = root / "scripts/aeneas_vote.sh"
    extra = ["--required"] if charon_required else []
    p = subprocess.run(["bash", str(script), *extra], cwd=root)
    if p.returncode != 0:
        r.fail(f"aeneas_vote.sh exit {p.returncode}")
    elif charon_required:
        r.good("aeneas_vote.sh (Charon+Aeneas)")


def check_verus(root: Path, catalog: dict, r: Report, required: bool) -> None:
    print("== verus (optional unless --verus-required) ==")
    verus = find_verus(root)
    if not verus:
        msg = "verus binary not found (set VERUS= or install ~/.local/verus/verus-arm64-macos)"
        if required:
            r.fail(msg)
        else:
            print(f"skip  {msg}")
        return
    print(f"      verus={verus}")
    for pair in catalog["pairs"]:
        if pair.get("status") == "absent":
            continue
        script = pair.get("verus")
        if not script:
            continue
        if not (root / pair["twin"]).is_file():
            r.fail(f"{pair['id']}: twin missing, skip verus")
            continue
        p = subprocess.run(["bash", str(root / script)], cwd=root)
        if p.returncode != 0:
            msg = f"verus {pair['id']} exit {p.returncode}"
            if required:
                r.fail(msg)
            else:
                print(f"warn  {msg} (not fatal; pass --verus-required to fail)")
        else:
            r.good(f"verus {pair['id']}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lint", action="store_true")
    ap.add_argument("--twins", action="store_true")
    ap.add_argument("--clones", action="store_true")
    ap.add_argument("--scripts", action="store_true")
    ap.add_argument("--models", action="store_true")
    ap.add_argument(
        "--extract",
        action="store_true",
        help="include-crate cargo test + Charon/Aeneas when installed",
    )
    ap.add_argument(
        "--extract-required",
        action="store_true",
        help="fail if Charon/Aeneas are missing",
    )
    ap.add_argument("--verus", action="store_true")
    ap.add_argument(
        "--verus-required",
        action="store_true",
        help="fail if the verus binary is missing",
    )
    ap.add_argument(
        "--strict",
        action="store_true",
        help="fail on catalog status=absent gaps (missing twins)",
    )
    ap.add_argument(
        "--ci",
        action="store_true",
        help="lint + clones + twins + scripts + models + include-crate extract (no Verus/Charon)",
    )
    ap.add_argument(
        "--all",
        action="store_true",
        help="--ci plus Verus when installed",
    )
    args = ap.parse_args()
    selected = any(
        [
            args.lint,
            args.twins,
            args.clones,
            args.scripts,
            args.models,
            args.extract,
            args.extract_required,
            args.verus,
            args.verus_required,
            args.ci,
            args.all,
        ]
    )
    if not selected:
        args.ci = True

    root = Path(__file__).resolve().parents[2]
    catalog = json.loads((root / "scripts/formal/catalog.json").read_text(encoding="utf-8"))
    r = Report()

    run_ci = args.ci or args.all
    if args.lint or run_ci:
        check_lint(root, catalog, r)
    if args.clones or run_ci:
        check_clones(root, catalog, r)
    if args.twins or run_ci:
        check_twins(root, catalog, r, args.strict)
    if args.scripts or run_ci:
        check_scripts(root, catalog, r, args.strict)
    if args.models or run_ci:
        check_models(root, catalog, r)
    if args.extract or args.extract_required or run_ci:
        check_extract(
            root,
            r,
            want_charon=args.extract or args.extract_required or args.all,
            charon_required=args.extract_required,
        )
    if args.verus or args.verus_required:
        check_verus(root, catalog, r, args.verus_required)
    elif args.all:
        # Optional: try Verus, do not fail the glue on a missing toolchain.
        check_verus(root, catalog, r, required=False)

    print()
    print(
        f"summary: {len(r.ok)} ok, {len(r.gaps)} gap, {len(r.failed)} fail"
    )
    if r.gaps and not args.strict:
        print("gaps are recorded twins (status=absent); pass --strict to fail on them")
    return 1 if r.failed else 0


if __name__ == "__main__":
    sys.exit(main())
