# Runbook — verification gates (RFC-0187 → RFC-0203)

**Every gate is blocking, self-verifying, and hang-fatal.** A gate proves
its own redness: `--selftest` plants each sabotage and must catch it all;
a mutation-blind gate is a red job (`.github/workflows/verification-gates.yml`
runs gate twice + `cmp` + selftest greps).

## Gate suite

| Gate | Script | Selftest | Pins |
|---|---|---|---|
| exhaustive | `pedradb-world --bin gate_exhaustive` | 4/4 | 66-schedule grant space, per-schedule oracle |
| seed-ratchet | `pedradb-world --bin gate_seed_ratchet` | 4/4 | pinned PCT seeds replay, file only grows |
| crash | `pedradb-world --bin gate_crash_injection` | 9/9 | every fallible-op index injected, fail-closed oracle |
| barrier | `scripts/check_barrier_floor.py` | 2/2 | barrier sites pinned (static == pinned, dynamic tie) |
| coverage | `pedradb-world --bin gate_coverage_floor` | 4/4 | pinned-seed interleaving union |
| ledger | `scripts/check_ledger_consistency.py` | 2/2 | layer ledger vs formal catalog |
| depth-floor | `scripts/check_depth_floor.py` | 5/5 | ladder floors + registry + residuals + `floor_count` |
| product-floor | `scripts/check_product_floor.py` | 4/4 | D1/R1/T1/C1 only up; ∀ theorem, no sorry |
| twin-contracts | `scripts/check_twin_contracts.py` | 6/6 | every `count` row bound: twin drives prod fn + annotation; TSV machine-emitted |
| inventory-terminal | `scripts/check_inventory_terminal.py` | 6/6 | ledger inventory terminal: `count` (registered) or `deferido` dated; never `todo` |

Lean side: `scripts/lean_extracts.sh --required` gates on
`derive_count_annotations.py --check` (CountDerived.lean + twin_contracts.tsv
in sync with extracts/registry) before the lake build.

## Movimento de linha — cadence rite for a NEW count pair (RFC-0203 P2.1)

A new kernel pair enters the count tier in ONE commit, or the gates are
red. The rite, in order:

1. **Theorem** — ∀, no `sorry`, over the Aeneas extract (or the Work.io
   algebra for barrier counts); never ns, never physical durability
   (RFC-0187: those are dated measurements/anchors, not theorems).
2. **Registry row** — kind `count` in `scripts/ratchet/close_proofs.tsv`
   (one per catalog pair; orthogonal to close/atom) +
   `floor_count` moves UP + `scripts/formal/residuals.json`
   `proof_depth.count` == live, same commit (depth-floor pins all three).
3. **Twin contract** — the twin test that drives the production fn goes
   into `TWIN` in `scripts/ratchet/derive_count_annotations.py`; rerun
   the tool (emits `twin_contracts.tsv` + `CountDerived.lean`). Without
   the TWIN entry the tool refuses to emit and twin-contracts is RED;
   a twin that mocks the unit under test is RED.
4. **Inventory line** — the ledger "Escada de contagem" table gets the
   pair's row with status `count` (or `deferido` + date + reason) in the
   same commit. A `todo` row in the ledger, a registered theorem without
   its row, or a row without its `catalog:<id>` — inventory-terminal RED.
5. **Anchor (only if a new host class)** — `scripts/ratchet/host_anchors.tsv`
   row dated + sourced + honest quiet/DIAG label, consumed by
   `crates/pedradb-core/tests/host_anchor_table.rs`. Counts stay
   class-independent (WorkIo any-class theorems); the anchor only fills
   the ns cost.

`todo` lives in the RFC's slice list, never in the ledger: the ledger is
the terminal record, the RFC is the plan.

### Mirror retirement — rite for retiring a hand count mirror (RFC-0204)

An enrolled pair's hand mirror (`*Count.lean`) retires to a
machine-emitted `*Derived.lean` in ONE commit, or the gates are red.
The rite, in order:

1. **Twin shape** — declare the pair in `TWIN_SHAPES` of
   `scripts/ratchet/derive_count_annotations.py`: shape vocabulary
   (proof template), out file, kernel lib/ns, twin def names, the
   REGISTERED bound theorem (name byte-stable), the bridges file, and
   `anchor_defs` (def names that must still exist in the pair's
   extracts). Emission validates shape fns are ENROLLED, anchors are
   present, and the emitted theorem name equals the registered count
   row — a stale shape fails `--check`, so the mirror cannot rot.
2. **Registry moves lean_file only** — the theorem name and entry stay
   byte-stable; only the `lean_file` column walks to the generated
   `*Derived.lean`.
3. **Bridges file** — `git mv` the old `*Count.lean` to
   `*Bridges.lean` and strip the moved twins; the semantic bridges
   stay HUMAN (zero `sorry`, ≥1 `^theorem`). A bridge the emitted
   bound composes (e.g. `saturating_add_val_le`) becomes PUBLIC.
4. **Build wiring** — swap the old lib name for `*Bridges` +
   `*Derived` in BOTH `formal/aeneas/lean/lakefile.toml` and the
   `COMPOSE` list of `scripts/lean_extracts.sh`; rerun the tool
   (emits the derived file + flips the contract annotation); gates +
   lean `--required` + the pair's Rust twin test green with the twin
   BYTE-UNEDITED (continuity proof: the registered quota neither
   renamed nor reshaped).
