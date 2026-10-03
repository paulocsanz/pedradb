#!/usr/bin/env python3
"""Public-mirror hygiene gate.

Fails when the public tree carries internal scaffolding or references to
documents that live only in the development tree. Run by CI on every push:

    python3 scripts/check_public_hygiene.py

Exit 0 = clean; exit 1 = violations printed.
"""
from __future__ import annotations

import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Cross-repo names from the dev environment must not leak as file names
# inside shipped directories (crates/, formal/, docs/, scripts/). This is
# fail-loud, not fail-delete: a hit means a human renames the file or
# the manifest gains it — the sync never silently drops source files
# (the caixote_kernel.rs scar, 2026-10-03).
FORBIDDEN_NAME_STEMS = ("caixote", "fonte", "janela")

# Generated proof output never ships: it is rebuildable from the
# extracts and can carry stale in-flight sorries.
FORBIDDEN_FORMAL_SUBDIRS = ("out",)

# The only docs the public mirror ships; everything else in docs/ is internal.
PUBLIC_DOCS = ("architecture.md", "benchmarks.md", "metrics.md", "verification.md")

# The public tree is DECLARED, not derived: only these root entries ship.
# The sync prunes to this manifest and the CI gate enforces it, so the
# mirror's surface is one auditable list instead of an exclusion net.
# Adding an entry here is a product decision: it ships to everyone.
PUBLIC_ROOT_DIRS = frozenset(
    {".github", "crates", "docs", "examples", "formal", "scripts"}
)
PUBLIC_ROOT_FILES = frozenset(
    {
        ".gitignore",
        ".public-mirror",  # written by sync; engages pedra_formal.py mirror mode (RFC-0331)
        "AGENTS.md",
        "Cargo.lock",
        "Cargo.toml",
        "LICENSE",
        "README.md",  # replaced by scripts/public_assets/README.md at sync
        "SECURITY.md",
        "clippy.toml",
        "deny.toml",
        "lint.log",  # README links it as the verification snapshot
        "rust-toolchain.toml",
    }
)

# The scripts/ surface the public mirror ships (RFC-0331 "proof
# reproduction assets"): the CI gates, their data files, and the
# per-kernel proof drivers named by the shipped source and docs.
# Everything else under scripts/ is internal lab tooling (bench
# harnesses, QEMU/TCG guests, RFC entrypoints, soak drivers) and must
# not ship. This allowlist is the single source of truth: the sync
# prunes with it and the CI hygiene gate enforces it, so the two can
# never drift apart.
PUBLIC_SCRIPT_PREFIXES = ("aeneas_", "verus_", "kani_", "lean_")

PUBLIC_SCRIPT_FILES = {
    # public CI + sync gates
    "check_public_hygiene.py",
    "ci_ratchet.py",
    # named by public docs (docs/verification.md, README)
    "pedra_formal.sh",
    "lean_extracts.sh",
    "mutation_fuzzer.py",
    "reproduce_lean_proofs.sh",
    "reproduce-scale.sh",
    # run by reproduce_lean_proofs.sh
    "check_lean_sorries_and_axioms.py",
    "sel4_gap.py",
    # executed by sel4_gap.py --gate (CHEAP_GATES list)
    "check_barrier_floor.py",
    "check_compose_floor.py",
    "check_depth_floor.py",
    "check_inventory_terminal.py",
    "check_ledger_consistency.py",
    "check_no_prod_time_spawn.py",
    "check_product_floor.py",
    "check_seam_inventory.py",
    "check_twin_contracts.py",
    # data file read by check_seam_inventory.py
    "seam_inventory_v1.json",
    # owner scripts registered in formal/residuals.json (R-unsafe-*); the
    # lint fails if a residual's script is not a file
    "capi-asan.sh",
    "miri-unsafe-islands.sh",
    # read AND executed by the shipped pedradb-world unit test
    # tcg_guest_status_script_names_kernel (crates/pedradb-world/src/lib_kernel.rs)
    "tcg_guest_status.sh",
    # the parameterized proof driver the generated aeneas_/verus_/lean_
    # shims exec (scripts/gen_proof_shims.py is internal-only tooling)
    "proof.sh",
    # cross-reference gate, run by the public CI hygiene job
    "check_xrefs.py",
}

# scripts/formal/ ships wholesale (pedra_formal.py, catalog.json,
# residuals.json, the lint's own tests) except caches.
PUBLIC_SCRIPT_DIRS = ("formal",)

# Machine-emitted registries and floors read by the gates above.
PUBLIC_SCRIPT_RATCHET = {
    "barrier_sites.tsv",        # check_barrier_floor.py
    "close_proofs.tsv",         # check_depth_floor.py, check_inventory_terminal.py, check_twin_contracts.py
    "compose_floor.tsv",        # check_compose_floor.py
    "derive_count_annotations.py",  # emitter for the ratchet registries (check_twin_contracts.py)
    "host_anchors.tsv",         # crates/pedradb-core/tests/host_anchor_table.rs
    "lean_axioms_catalog.tsv",  # check_lean_sorries_and_axioms.py
    "lean_axioms_ceiling.json", # check_lean_sorries_and_axioms.py
    "product_guarantees.tsv",   # check_product_floor.py
    "proof_depth.tsv",          # check_depth_floor.py
    "sel4_gap_floors.json",     # sel4_gap.py
    "twin_contracts.tsv",       # check_twin_contracts.py
}


def script_is_public(rel: str) -> bool:
    """Whether scripts/<rel> ships in the public mirror."""
    parts = rel.split("/")
    if "__pycache__" in parts:
        return False
    if parts[0] in PUBLIC_SCRIPT_DIRS:
        return True
    if parts[0] == "ratchet":
        return len(parts) == 2 and parts[1] in PUBLIC_SCRIPT_RATCHET
    if len(parts) != 1:
        return False
    name = parts[0]
    return name in PUBLIC_SCRIPT_FILES or (
        name.startswith(PUBLIC_SCRIPT_PREFIXES) and name.endswith(".sh")
    )


def prune_scripts(root: Path) -> list[str]:
    """Delete every file under <root>/scripts the public mirror does not
    ship, then sweep the directories left empty. Returns the removed
    paths relative to scripts/. Survivors are exactly what
    script_is_public accepts — the sync and the CI gate share one list."""
    scripts_dir = root / "scripts"
    removed = []
    if not scripts_dir.is_dir():
        return removed
    for p in sorted(scripts_dir.rglob("*")):
        rel = p.relative_to(scripts_dir).as_posix()
        if p.is_file() and not script_is_public(rel):
            removed.append(rel)
            p.unlink()
    for p in sorted(scripts_dir.rglob("*"), key=lambda q: len(q.parts), reverse=True):
        if p.is_dir() and not any(p.iterdir()):
            p.rmdir()
    return removed


def prune_root(root: Path) -> list[str]:
    """Delete every root entry the public manifest does not declare.
    Returns the removed names. Never touches .git."""
    removed = []
    for p in sorted(root.iterdir()):
        if p.name == ".git":
            continue
        if p.is_dir() and p.name not in PUBLIC_ROOT_DIRS:
            removed.append(f"{p.name}/")
            shutil.rmtree(p)
        elif p.is_file() and p.name not in PUBLIC_ROOT_FILES:
            removed.append(p.name)
            p.unlink()
    return removed

# Markdown links pointing at internal-only trees (relative to any doc).
INTERNAL_LINK_RE = re.compile(
    r"\]\((?:\.\./)*(?:docs/)?"
    r"(?:rfc|audits|reports|research|references|runbooks)/[^)]+\)"
)

# Sibling working docs that were removed from the mirror.
REMOVED_SIBLINGS = (
    "montanhadb",
    "montanha-layering-dcs-on-store",
    "distribution-design",
    "engine-landscape-and-ideal-path",
    "fdb-limitations-analysis",
    "dst-seams",
    "live-leadership-and-patroni-shaped-ha",
    "TRAJETORIA",
    "apply-and-raft",
    "positioning",
)
SIBLING_LINK_RE = re.compile(
    r"\]\((?:\.\./)*(?:docs/)?(" + "|".join(REMOVED_SIBLINGS) + r")\.md\)"
)


def main() -> int:
    bad = False

    # Root surface: exactly the declared manifest, nothing extra,
    # nothing missing.
    present_dirs = set()
    present_files = set()
    for p in ROOT.iterdir():
        if p.name == ".git":
            continue
        (present_dirs if p.is_dir() else present_files).add(p.name)
    for name in sorted(present_dirs - PUBLIC_ROOT_DIRS):
        print(f"HYGIENE FAIL: root directory not in the public manifest: {name}/")
        bad = True
    for name in sorted(present_files - PUBLIC_ROOT_FILES):
        print(f"HYGIENE FAIL: root file not in the public manifest: {name}")
        bad = True
    for name in sorted(PUBLIC_ROOT_DIRS - present_dirs):
        print(f"HYGIENE FAIL: declared public directory missing: {name}/")
        bad = True
    for name in sorted(PUBLIC_ROOT_FILES - present_files):
        print(f"HYGIENE FAIL: declared public file missing: {name}")
        bad = True

    # The mirror-mode marker is load-bearing: without it pedra_formal.py
    # runs the internal RFC-corpus coherence checks against a tree that
    # (correctly) has no docs/rfc and fails 30+ times. Name the cause.
    if ".public-mirror" not in present_files:
        print(
            "HYGIENE FAIL: .public-mirror marker missing — the formal lint "
            "cannot engage mirror mode (RFC-0331); re-run scripts/sync_public_repo.sh"
        )

    docs_dir = ROOT / "docs"
    if docs_dir.is_dir():
        for entry in docs_dir.iterdir():
            if entry.name not in PUBLIC_DOCS and entry.name != "README.md":
                print(f"HYGIENE FAIL: non-public doc shipped: docs/{entry.name}")
                bad = True

    scripts_dir = ROOT / "scripts"
    if scripts_dir.is_dir():
        shipped = set()
        for p in scripts_dir.rglob("*"):
            if not p.is_file() or ".git" in p.parts:
                continue
            rel = p.relative_to(scripts_dir).as_posix()
            shipped.add(rel)
            if not script_is_public(rel):
                print(f"HYGIENE FAIL: internal script shipped: scripts/{rel}")
                bad = True
        required = sorted(
            PUBLIC_SCRIPT_FILES
            | {f"ratchet/{n}" for n in PUBLIC_SCRIPT_RATCHET}
            | {
                "formal/pedra_formal.py",
                "formal/catalog.json",
                "formal/residuals.json",
            }
        )
        for rel in required:
            if rel not in shipped:
                print(f"HYGIENE FAIL: required public script missing: scripts/{rel}")
                bad = True

    for stray in FORBIDDEN_FORMAL_SUBDIRS:
        if (ROOT / "formal" / stray).is_dir():
            print(f"HYGIENE FAIL: generated formal output shipped: formal/{stray}/")
            bad = True

    for p in ROOT.rglob("*"):
        if not p.is_file() or ".git" in p.parts:
            continue
        stem = p.name.lower()
        if any(s in stem for s in FORBIDDEN_NAME_STEMS):
            print(f"HYGIENE FAIL: cross-repo file name leaked: {p.relative_to(ROOT)}")
            bad = True

    for p in list(ROOT.glob("*.md")) + list((ROOT / "docs").glob("*.md")):
        text = p.read_text(encoding="utf-8", errors="replace")
        rel = p.relative_to(ROOT)
        for m in INTERNAL_LINK_RE.finditer(text):
            print(f"HYGIENE FAIL: {rel} links internal-only path: {m.group(0)}")
            bad = True
        for m in SIBLING_LINK_RE.finditer(text):
            print(f"HYGIENE FAIL: {rel} links removed sibling doc: {m.group(0)}")
            bad = True

    if not bad:
        print("hygiene: clean (no internal scaffolding, no dangling internal links)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
