#!/usr/bin/env python3
"""RFC-0199 P2.1 — mechanical derivation of per-step cost annotations
over the Aeneas extracts ("cost-instrumented extract" as a repo-maintained
post-processor; the pinned Aeneas fork itself stays untouched).

For every ENROLLED extract function the tool counts, textually and
deterministically, the work-relevant nodes of ONE unfolding of the
function and emits `formal/aeneas/lean/CountDerived.lean` — Nat defs
plus theorems (equation + positivity) that compile in the same lake
build as the extracts. Counters (each matches exactly what the regex
says, nothing more):

  leaf_calls    occurrences of in-file `axiom` names (untranslated
                external work — the IO/iterator leaves)
  local_calls   occurrences of OTHER in-file `def` names (constructor-
                level calls; each counts 1 regardless of callee cost)
  dispatches    `match` + `if` keywords (branch dispatches)
  cmp_ops       machine comparisons: `<=` `>=` `<` `>` (shifts `<<<`
                stripped first) `≠` `=` (excluding `:=` `=>` `==`)
  arith_ops     `<<<` plus spaced ` + ` ` - ` ` * `
  nested_calls  occurrences of OTHER ENROLLED function names — a whole
                sub-loop run, EXCLUDED from step_work (charged by its
                own registered count theorem, never folded flat)
  self_calls    occurrences of the function's own name (incl. `.body`)
                — EXCLUDED from step_work: recursion for `X_loop` runs
                through the Aeneas `loop` combinator and the registered
                theorem, not a flat constant

step_work = leaf_calls + local_calls + dispatches + cmp_ops + arith_ops.
Comments (`--`, `/- -/`, `/-- -/`) are stripped before counting.

Modes:
  (default)  regenerate formal/aeneas/lean/CountDerived.lean in place
  --check    exit 1 (with a diff) if the on-disk file is stale
  --lean-dir DIR   read extracts from DIR (default: real extracts)
  --out FILE       write to FILE instead of CountDerived.lean

Re-run after any extract change; `scripts/lean_extracts.sh` gates on
`--check` before building, so a stale annotation file fails the lean
gate.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
LEAN_DIR_DEFAULT = REPO / "formal" / "aeneas" / "lean"
OUT_DEFAULT = LEAN_DIR_DEFAULT / "CountDerived.lean"

# (extract file, function name, registered catalog pair it serves).
# The per-unfolding body of `X_loop` is the `X_loop.body` def.
ENROLLED: list[tuple[str, str, str]] = [
    ("FlushKernel.lean", "auto_flush_due", "catalog:auto_flush_due"),
    ("ScanKernel.lean", "scan_kernel.scan_reads_file", "catalog:scan_guard"),
    ("LsmR1Kernel.lean", "lsm_compact_src_loop", "catalog:lsm_compact"),
    ("LsmR1Kernel.lean", "lsm_compact_inner_loop", "catalog:lsm_compact"),
    ("ProbeOrderKernel.lean", "probe_order_covering_loop", "catalog:probe_order_covering"),
    ("ScaleKernel.lean", "level_count_loop", "catalog:scale_predict"),
]

DECL_RE = re.compile(r"^(?:axiom|def)\s+(\S+)", re.MULTILINE)

# RFC-0203 P0.1 — twin contracts. Every registered `count` row must name
# the twin test that drives the production fn; a count row without an
# entry here fails emission (the contract lands in the SAME commit as the
# row, machine-checked by scripts/check_twin_contracts.py).
# catalog pair -> (twin test path, production fn the twin drives).
TWIN: dict[str, tuple[str, str]] = {
    "catalog:lsm_compact": ("crates/pedradb-core/tests/lsm_compact_count.rs", "lsm_compact"),
    "catalog:probe_order_covering": ("crates/pedradb-core/src/probe_order_kernel.rs", "probe_order_covering"),
    "catalog:scale_predict": ("crates/pedradb-core/tests/scale_ladder_count.rs", "point_get_probes"),
    "catalog:wal_commit_plan": ("crates/pedradb-core/tests/wal_commit_work_count.rs", "wal_commit_plan"),
    "catalog:auto_flush_due": ("crates/pedradb-core/tests/flush_amort_count.rs", "auto_flush_due"),
    "catalog:scan_guard": ("crates/pedradb-core/tests/scan_decision_count.rs", "scan_reads_file"),
    "catalog:bloom_may_contain": ("crates/pedradb-core/tests/bloom_probe_count.rs", "may_contain"),
}
REGISTRY = REPO / "scripts" / "ratchet" / "close_proofs.tsv"
CONTRACTS_TSV = REPO / "scripts" / "ratchet" / "twin_contracts.tsv"

# RFC-0204 P0.1 — retired mirrors. Per pair, a DECLARED twin shape drives
# emission of the Nat mirror layer (iteration twins + the pair's
# REGISTERED bound theorem, name byte-stable) into a generated
# `*Derived.lean`. Emission validates the shape against the SAME parse
# that derives step_work: the shape's loop fns must be the pair's
# ENROLLED fns, the drain loop must still make exactly one nested
# enrolled call, and the emitted theorem name must equal the pair's
# registered count theorem — a stale shape fails emission, so the mirror
# cannot rot silently (lean_extracts.sh gates on --check).
# Shape vocabulary (proof templates: induction/omega, rfl):
#   drain_over_levels — inner consume loop (one step per remaining
#   entry) + outer drain (per level: consume(level_len) + 1 bookkeeping).
TWIN_SHAPES: dict[str, dict[str, str]] = {
    "catalog:lsm_compact": {
        "shape": "drain_over_levels",
        "out": "LsmCompactDerived.lean",
        "kernel_lib": "LsmR1Kernel",
        "kernel_ns": "pedra_aeneas_lsm_r1_kernel",
        "state_type": "LsmState",
        "drain_fn": "lsm_compact_src_loop",
        "consume_fn": "lsm_compact_inner_loop",
        "steps_def": "lsm_compact_steps",
        "level_len_def": "lsm_level_len",
        "entries_def": "lsm_entries_below",
        "drain_steps_def": "lsm_compact_src_steps",
        "steps_le_thm": "lsm_compact_steps_le",
        "bound_thm": "lsm_compact_work_bound",
        "bridges_file": "LsmCompactBridges.lean",
    },
}
BLOCK_END_RE = re.compile(
    r"^(?=/-- |/- |@\[|^axiom |^def |^theorem |^inductive |^namespace |^end |^section )",
    re.MULTILINE,
)


def strip_comments(text: str) -> str:
    text = re.sub(r"/--.*?-/ ", "", text, flags=re.DOTALL)
    text = re.sub(r"/-.*?-/", "", text, flags=re.DOTALL)
    text = re.sub(r"--[^\n]*", "", text)
    return text


def def_block(clean: str, name: str) -> str:
    """Text of `def <name> ...` up to the next top-level declaration."""
    m = re.search(rf"^def {re.escape(name)}(?=[\s(\n:])", clean, re.MULTILINE)
    if m is None:
        raise SystemExit(f"FAIL  def {name} not found")
    rest = clean[m.start() :]
    first_nl = rest.index("\n") + 1
    tail = rest[first_nl:]
    nxt = re.search(r"^(?=/-- |/- |@\[|axiom |def |theorem |inductive |namespace |end |section )",
                    tail, re.MULTILINE)
    stop = first_nl + nxt.start() if nxt else len(rest)
    return rest[:stop]


def count_occurrences(text: str, name: str) -> int:
    return len(re.findall(rf"(?<![\w.]){re.escape(name)}(?![\w.])", text))


def derive(lean_dir: Path) -> list[dict]:
    rows = []
    for extract, fn, pair in ENROLLED:
        path = lean_dir / extract
        if not path.is_file():
            raise SystemExit(f"FAIL  enrolled extract missing: {path}")
        src = path.read_text()
        clean = strip_comments(src)
        axioms = re.findall(r"^axiom\s+(\S+)", clean, re.MULTILINE)
        defs = re.findall(r"^def\s+(\S+)", clean, re.MULTILINE)

        # the per-unfolding body of a loop fn is its `.body` def; work is
        # counted from the body (after `:=`), never the signature
        body_fn = fn if re.search(rf"^def {re.escape(fn)}\.body(?=\s)", clean, re.MULTILINE) is None \
            else f"{fn}.body"
        block = def_block(clean, body_fn)
        body = block[block.index(":="):] if ":=" in block else block

        enrolled_names = {f for (_, f, _) in ENROLLED} | {f"{f}.body" for (_, f, _) in ENROLLED}
        leaf = sum(count_occurrences(body, a) for a in axioms)
        nested = sum(count_occurrences(body, f) for f in enrolled_names
                     if f not in {fn, f"{fn}.body"})
        local = sum(count_occurrences(body, d) for d in defs
                    if d not in enrolled_names and d not in {fn, f"{fn}.body", body_fn})
        self_calls = count_occurrences(body, fn) + (
            count_occurrences(body, f"{fn}.body") if body_fn != fn else 0)
        dispatches = len(re.findall(r"\b(?:match|if)\b", body))
        no_shift = body.replace("<<<", "")
        cmp_ops = (len(re.findall(r"<=|>=", no_shift))
                   + len(re.findall(r"(?<![\w<>=])<(?![<=])", no_shift))
                   + len(re.findall(r"(?<![<>=])>(?![>=])", no_shift))
                   + no_shift.count("≠")
                   + len(re.findall(r"(?<![:\-<>=])=(?![=>])", no_shift)))
        arith = body.count("<<<") + len(re.findall(r" \+ | - | \* ", body))
        step = leaf + local + dispatches + cmp_ops + arith
        rows.append(dict(extract=extract, fn=fn, pair=pair, leaf=leaf, local=local,
                         dispatches=dispatches, cmp=cmp_ops, arith=arith,
                         nested=nested, self_calls=self_calls, step=step))
    return rows


def sanitize(fn: str) -> str:
    return fn.replace(".", "_")


def render(rows: list[dict]) -> str:
    out = []
    out.append("-- AUTO-GENERATED by scripts/ratchet/derive_count_annotations.py")
    out.append("-- RFC-0199 P2.1 — mechanically derived per-step cost annotations")
    out.append("-- over the Aeneas extracts. DO NOT EDIT: regenerate with")
    out.append("--   python3 scripts/ratchet/derive_count_annotations.py")
    out.append("-- (scripts/lean_extracts.sh gates on --check before building).")
    out.append("-- step_work = leaf_calls (in-file axioms) + local_calls (in-file")
    out.append("-- defs) + dispatches (match/if) + cmp_ops + arith_ops; nested")
    out.append("-- enrolled-loop calls and self-recursion are counted separately")
    out.append("-- and EXCLUDED — a sub-loop is charged by its own registered")
    out.append("-- count theorem, never folded into a flat constant.")
    for mod in dict.fromkeys(e.removesuffix("Kernel.lean") for e, _, _ in ENROLLED):
        out.append(f"import {mod}Kernel")
    out.append("")
    out.append("namespace pedra_aeneas_count_derived")
    out.append("")
    for r in rows:
        n = sanitize(r["fn"])
        out.append(f"/-- `{r['fn']}` ({r['extract']}, {r['pair']}): derived one-unfolding")
        out.append(f"     cost — leaf={r['leaf']} local={r['local']} dispatch={r['dispatches']}")
        out.append(f"     cmp={r['cmp']} arith={r['arith']} (nested={r['nested']},")
        out.append(f"     self={r['self_calls']} excluded). AUTO-GENERATED. -/")
        FIELD_KEYS = {"leaf_calls": "leaf", "local_calls": "local",
                      "dispatches": "dispatches", "cmp_ops": "cmp",
                      "arith_ops": "arith", "nested_calls": "nested",
                      "self_calls": "self_calls"}
        for field, key in FIELD_KEYS.items():
            out.append(f"def {n}_{field} : Nat := {r[key]}")
        out.append(f"def {n}_step_work : Nat := {r['step']}")
        out.append(f"theorem {n}_step_work_eq : {n}_step_work = {n}_leaf_calls + {n}_local_calls + {n}_dispatches + {n}_cmp_ops + {n}_arith_ops := by rfl")
        if r["step"] > 0:
            out.append(f"theorem {n}_step_work_positive : 0 < {n}_step_work := by decide")
        else:
            out.append(f"theorem {n}_step_work_zero : {n}_step_work = 0 := by rfl")
        out.append("")
    total = " + ".join(f"{sanitize(r['fn'])}_step_work" for r in rows)
    out.append(f"/-- Every enrolled extract function does at least one unit of")
    out.append(f"     mechanically counted work per unfolding. -/")
    out.append(f"theorem all_enrolled_step_work_positive : 0 < {total} := by decide")
    out.append("")
    out.append("end pedra_aeneas_count_derived")
    return "\n".join(out) + "\n"


def count_registry_rows() -> list[dict[str, str]]:
    """Registered `count` rows of close_proofs.tsv (the credit registry)."""
    rows = []
    for line in REGISTRY.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) == 5 and parts[0] == "count":
            rows.append(dict(zip(("kind", "catalog_id", "theorem", "lean_file", "entry"), parts)))
    return rows


def render_contracts(rows: list[dict[str, str]]) -> str:
    """RFC-0203 P0.1 — one contract row per registered count row: the twin
    test that drives the production fn plus the annotation carrying the
    work bound (derived `*_step_work` defs for enrolled pairs; the
    registered hand theorem for pairs whose bound lives in a hand-written
    mirror — WorkIo / BloomCount)."""
    enrolled_by_pair: dict[str, list[str]] = {}
    for (_, fn, pair) in ENROLLED:
        enrolled_by_pair.setdefault(pair, []).append(sanitize(fn) + "_step_work")
    out = [
        "# RFC-0203 P0.1 — twin contracts: every registered `count` row bound",
        "# to the twin test that drives the production fn and the annotation",
        "# that carries its work bound. AUTO-GENERATED by",
        "# scripts/ratchet/derive_count_annotations.py — DO NOT EDIT.",
        "# Columns (tab-separated):",
        "#   catalog_id  theorem  lean_file  twin_test  prod_fn  annotation  annotation_file",
        "# annotation: space-separated CountDerived.lean `*_step_work` defs for",
        "# enrolled pairs (mechanically derived); the registered hand theorem",
        "# for hand-written mirrors (WorkIo / BloomCount).",
        "# Gate: scripts/check_twin_contracts.py (missing / mocking twin / stale",
        "# annotation / hand-edited TSV = RED).",
    ]
    for r in rows:
        pair = r["catalog_id"]
        if pair not in TWIN:
            raise SystemExit(
                f"FAIL  count row {pair} ({r['theorem']}) has no twin contract — "
                "add the twin test + prod fn to TWIN in derive_count_annotations.py "
                "in the SAME commit as the registry row"
            )
        twin_test, prod_fn = TWIN[pair]
        if pair in TWIN_SHAPES:
            # RFC-0204 retired mirror: the work bound is MACHINE-EMITTED into
            # the pair's generated *Derived.lean under the registered name;
            # the contract binds that emission (not the hand file).
            ann = TWIN_SHAPES[pair]["bound_thm"]
            ann_file = f"formal/aeneas/lean/{TWIN_SHAPES[pair]['out']}"
        elif pair in enrolled_by_pair:
            ann, ann_file = " ".join(enrolled_by_pair[pair]), "formal/aeneas/lean/CountDerived.lean"
        else:
            ann, ann_file = r["theorem"], r["lean_file"]
        out.append("\t".join([pair, r["theorem"], r["lean_file"], twin_test, prod_fn, ann, ann_file]))
    return "\n".join(out) + "\n"


def validate_shape(pair: str, shape: dict, rows: list[dict], count_rows: list[dict]) -> None:
    enrolled = {fn for (_, fn, p) in ENROLLED if p == pair}
    for key in ("drain_fn", "consume_fn"):
        if shape[key] not in enrolled:
            raise SystemExit(f"FAIL  twin shape {pair}: {key}={shape[key]!r} is not an ENROLLED fn of this pair")
    drain = next((r for r in rows if r["fn"] == shape["drain_fn"]), None)
    if drain is None:
        raise SystemExit(f"FAIL  twin shape {pair}: drain fn {shape['drain_fn']} not parsed from the extract")
    if drain["nested"] != 1:
        raise SystemExit(
            f"FAIL  twin shape {pair}: drain loop makes {drain['nested']} nested enrolled calls "
            "(shape declares 1 — the extract changed shape; re-adjudicate the twin)"
        )
    reg = next((r for r in count_rows if r["catalog_id"] == pair), None)
    if reg is None:
        raise SystemExit(f"FAIL  twin shape {pair}: no registered count row in {REGISTRY.name}")
    if reg["theorem"] != shape["bound_thm"]:
        raise SystemExit(
            f"FAIL  twin shape {pair}: emitted bound {shape['bound_thm']!r} != registered "
            f"{reg['theorem']!r} — the registered theorem name is byte-stable"
        )


def render_mirror(shape: dict) -> str:
    """drain_over_levels template: the Nat layer the hand mirror carried
    (twins + steps_le + the REGISTERED bound theorem), now emitted."""
    if shape["shape"] != "drain_over_levels":
        raise SystemExit(f"FAIL  unknown twin shape {shape['shape']!r}")
    s = shape
    return f"""-- AUTO-GENERATED by scripts/ratchet/derive_count_annotations.py
-- RFC-0204 P0.1 — retired mirror: the Nat count-twin layer of the
-- lsm_compact walk is MACHINE-EMITTED from the declared twin shape,
-- validated against the same parse that derives step_work (drain fn
-- makes exactly 1 nested enrolled call; emitted theorem name equals the
-- registered count row). DO NOT EDIT: regenerate with
--   python3 scripts/ratchet/derive_count_annotations.py
-- (scripts/lean_extracts.sh gates on --check before building). The
-- semantic bridges to the real extract stay HUMAN, by design, in
-- {s['bridges_file']}.
import Aeneas
import {s['kernel_lib']}
open Aeneas Aeneas.Std Result ControlFlow
open {s['kernel_ns']}

/-! ## Count twins (pure Nat, machine-emitted) -/

/-- Work twin of the compact inner loop: iterations while `remaining`
entries are left to consume (`remaining` is the loop's own decreasing
measure `src_len - i`) — one step per consumed entry. -/
def {s['steps_def']} : Nat → Nat
  | 0 => 0
  | remaining + 1 => 1 + {s['steps_def']} remaining

/-- Logical length (`len`) of level `lvl` of `s`; 0 when out of range. -/
def {s['level_len_def']} (s : {s['state_type']}) (lvl : Nat) : Nat :=
  match s.levels.val[lvl]? with
  | some l => l.len.val
  | none => 0

/-- Entries stored strictly below `depth` — the compact drain set. -/
def {s['entries_def']} (s : {s['state_type']}) : Nat → Nat
  | 0 => 0
  | lvl + 1 => {s['level_len_def']} s lvl + {s['entries_def']} s lvl

/-- Work twin of the whole compact walk: one inner pass over each source
level's stored entries plus one bookkeeping iteration per drained level. -/
def {s['drain_steps_def']} (s : {s['state_type']}) : Nat → Nat
  | 0 => 0
  | lvl + 1 => {s['steps_def']} ({s['level_len_def']} s lvl) + 1 + {s['drain_steps_def']} s lvl

/-- Inner twin bound: one iteration per remaining entry, no more. -/
theorem {s['steps_le_thm']} : ∀ (remaining : Nat),
    {s['steps_def']} remaining ≤ remaining := by
  intro remaining
  induction remaining with
  | zero => simp [{s['steps_def']}]
  | succ d ih => simp only [{s['steps_def']}]; omega

/-- RFC-0199 count (P0.2), machine-emitted since RFC-0204 P0.1: the
compact walk's work twin never exceeds the entries stored below the
target level plus one bookkeeping iteration per drained level — one-pass
over the stored data, independent of how often compact is called or how
big the deeper (untouched) levels are. -/
theorem {s['bound_thm']} : ∀ (s : {s['state_type']}) (depth : Nat),
    {s['drain_steps_def']} s depth ≤ {s['entries_def']} s depth + depth := by
  intro s depth
  induction depth with
  | zero => simp [{s['drain_steps_def']}, {s['entries_def']}]
  | succ lvl ih =>
    have h := {s['steps_le_thm']} ({s['level_len_def']} s lvl)
    simp only [{s['drain_steps_def']}, {s['entries_def']}]
    omega
"""


def render_all_mirrors(rows: list[dict], count_rows: list[dict]) -> dict[str, str]:
    """out-file name -> generated content, one per retired pair."""
    out: dict[str, str] = {}
    for pair, shape in TWIN_SHAPES.items():
        validate_shape(pair, shape, rows, count_rows)
        out[shape["out"]] = render_mirror(shape)
    return out


def main() -> int:
    args = sys.argv[1:]
    check = "--check" in args
    lean_dir = LEAN_DIR_DEFAULT
    out_file = OUT_DEFAULT
    if "--lean-dir" in args:
        lean_dir = Path(args[args.index("--lean-dir") + 1])
    if "--out" in args:
        out_file = Path(args[args.index("--out") + 1])

    rows = derive(lean_dir)
    content = render(rows)
    count_rows = count_registry_rows()
    contracts = render_contracts(count_rows)
    mirrors = render_all_mirrors(rows, count_rows)
    if check:
        ok = True
        on_disk = out_file.read_text() if out_file.is_file() else ""
        if on_disk != content:
            print("FAIL  CountDerived.lean stale — regenerate:")
            print(f"      python3 {Path(__file__).name}  (writes {out_file})")
            import difflib
            for line in list(difflib.unified_diff(on_disk.splitlines(), content.splitlines(),
                                                  "on-disk", "regenerated", lineterm=""))[:40]:
                print("      " + line)
            ok = False
        contracts_on_disk = CONTRACTS_TSV.read_text() if CONTRACTS_TSV.is_file() else ""
        if contracts_on_disk != contracts:
            print("FAIL  twin_contracts.tsv stale — regenerate:")
            print(f"      python3 {Path(__file__).name}  (writes {CONTRACTS_TSV})")
            import difflib
            for line in list(difflib.unified_diff(contracts_on_disk.splitlines(), contracts.splitlines(),
                                                  "on-disk", "regenerated", lineterm=""))[:40]:
                print("      " + line)
            ok = False
        for name, mirror in mirrors.items():
            mirror_file = lean_dir / name
            mirror_on_disk = mirror_file.read_text() if mirror_file.is_file() else ""
            if mirror_on_disk != mirror:
                print(f"FAIL  {name} stale — regenerate:")
                print(f"      python3 {Path(__file__).name}  (writes {mirror_file})")
                import difflib
                for line in list(difflib.unified_diff(mirror_on_disk.splitlines(), mirror.splitlines(),
                                                      "on-disk", "regenerated", lineterm=""))[:40]:
                    print("      " + line)
                ok = False
        if ok:
            print("ok    CountDerived.lean in sync with extracts")
            print("ok    twin_contracts.tsv in sync with the count registry")
            for name in mirrors:
                print(f"ok    {name} in sync with the declared twin shape")
        return 0 if ok else 1
    out_file.write_text(content)
    CONTRACTS_TSV.write_text(contracts)
    for name, mirror in mirrors.items():
        (lean_dir / name).write_text(mirror)
    print(f"ok    wrote {out_file} ({len(rows)} enrolled fns)")
    print(f"ok    wrote {CONTRACTS_TSV} ({len(count_rows)} twin contracts)")
    for name in mirrors:
        print(f"ok    wrote {lean_dir / name} (retired mirror)")
    for r in rows:
        print(f"      {r['fn']:<28} step_work={r['step']:>3} "
              f"(leaf={r['leaf']} local={r['local']} disp={r['dispatches']} "
              f"cmp={r['cmp']} arith={r['arith']} | nested={r['nested']} self={r['self_calls']})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
