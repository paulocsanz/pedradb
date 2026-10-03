#!/usr/bin/env python3
"""Cross-reference validator: every scripts/ path that anything names
must exist, and every script the formal registries name must exist.

The scar this closes (2026-10-03): docs/benchmarks.md pointed at a
scale-reproduce script that exists in no tree, and it shipped that way
because nothing checks that named scripts are real.

    python3 scripts/check_xrefs.py          # public surface (what ships)
    python3 scripts/check_xrefs.py --all    # + internal lab docs (strict)

The default mode scans exactly the surface the mirror ships (README,
public docs, crates, workflows, scripts) — a dangling name there ships
to readers. --all adds the internal trees (docs/rfc, docs/audits,
docs/status, ...) where historical documents legitimately name scripts
that were renamed or deleted later; failures there are reported but are
historical record, not shipping bugs.

Excluded from both: prose family prefixes ("scripts/verus_") and
dot-file fixtures the tests create and delete at runtime.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# A path-shaped mention of something under scripts/.
SCRIPT_REF = re.compile(r"scripts/([A-Za-z0-9_][A-Za-z0-9_./-]*)")

# Mentions that are patterns, not paths (f-strings, globs, placeholders).
NOT_A_PATH = re.compile(r"[@{}*]")

# The shipped surface (default mode): what the mirror carries.
PUBLIC_GLOBS = (
    "README.md",
    "docs/architecture.md",
    "docs/benchmarks.md",
    "docs/metrics.md",
    "docs/verification.md",
    "crates/**/*.rs",
    "examples/**/*.rs",
    ".github/workflows/*.yml",
    ".github/workflows/*.yaml",
    "scripts/**/*.py",
    "scripts/**/*.sh",
    "scripts/**/*.json",
    "scripts/**/*.tsv",
)

# Internal trees, scanned only with --all.
INTERNAL_GLOBS = (
    "*.md",
    "docs/**/*.md",
)


def trim(ref: str) -> str:
    ref = ref.rstrip(".,;:)\"'")
    return ref


HISTORICAL_MENTIONS = frozenset({
    # Retired Verus twins replaced by Aeneas/Lean (documented in d1/t1/c1 kernel headers)
    "verus_d1_modelo.sh",
    "verus_t1_modelo.sh",
    "verus_c1_modelo.sh",
    # Internal historical script mentions
    "pedra_explore.sh",
    "tesoura_pedra_smoke.sh",
    "blocked_residuals_status.sh",
    "linux_det_io_ci.sh",
    "qemu_subset_revalidate.sh",
    # Internal lab tooling / generators excluded from the public mirror (RFC-0331)
    "gen_proof_shims.py",
    "tcg_world_smoke.sh",
    "dst_campaign.sh",
    "swarm_physical_disk.sh",
    "fdb_side_shapes.sh",
    "race_job.sh",
    "ratchet/pct_seeds.txt",
    "ratchet/coverage_floor.tsv",
    "perf_calltree.py",
    "rocks_side_ycsb.sh",
    "rocksdb_parity_v0.sh",
    "public_assets/README.md",
    "sync_public_repo.sh",
})


def is_checkable(ref: str) -> bool:
    """Prose prefixes ("scripts/verus_") and runtime dot-file fixtures
    are mentions, not paths."""
    if NOT_A_PATH.search(ref):
        return False
    if ref.endswith("_"):
        return False
    if Path(ref).name.startswith("."):
        return False
    if ref in HISTORICAL_MENTIONS:
        return False
    return True


def main() -> int:
    strict = "--all" in sys.argv[1:]
    globs = PUBLIC_GLOBS + (INTERNAL_GLOBS if strict else ())
    bad = []

    # --- 1. prose/code mentions -----------------------------------------
    seen: set[str] = set()
    for pattern in globs:
        for p in ROOT.glob(pattern):
            if not p.is_file() or ".git" in p.parts or "target" in p.parts:
                continue
            text = p.read_text(encoding="utf-8", errors="replace")
            for m in SCRIPT_REF.finditer(text):
                ref = trim(m.group(1))
                if not is_checkable(ref):
                    continue
                seen.add(ref)
                if not (ROOT / "scripts" / ref).exists():
                    bad.append(f"{p.relative_to(ROOT)} names scripts/{ref} (no such file)")

    # --- 2. catalog registries ------------------------------------------
    catalog = json.loads((ROOT / "scripts/formal/catalog.json").read_text())
    for pair in catalog["pairs"]:
        pid = pair.get("id", "?")
        for field in ("aeneas", "verus"):
            s = pair.get(field)
            if s and not (ROOT / s).exists():
                bad.append(f"catalog pair {pid}: {field} script missing: {s}")
        for field in ("kernel", "twin"):
            s = pair.get(field)
            if s and not (ROOT / s).exists():
                bad.append(f"catalog pair {pid}: {field} source missing: {s}")

    # --- 3. residuals owner scripts -------------------------------------
    res = json.loads((ROOT / "scripts/formal/residuals.json").read_text())

    def walk(o):
        if isinstance(o, dict):
            for v in o.values():
                yield from walk(v)
        elif isinstance(o, list):
            for v in o:
                yield from walk(v)
        elif isinstance(o, str):
            yield o

    for s in walk(res):
        for m in SCRIPT_REF.finditer(s):
            ref = trim(m.group(1))
            if not is_checkable(ref):
                continue
            if not (ROOT / "scripts" / ref).exists():
                bad.append(f"residuals.json names scripts/{ref} (no such file)")

    mode = "strict (--all)" if strict else "public surface"
    if bad:
        for line in bad:
            print(f"XREF FAIL: {line}")
        print(f"xrefs[{mode}]: {len(bad)} dangling reference(s) over {len(seen)} named")
        return 1
    print(f"xrefs[{mode}]: clean ({len(seen)} named script paths all exist)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
