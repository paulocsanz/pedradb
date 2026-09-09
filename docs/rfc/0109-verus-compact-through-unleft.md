# RFC: 0109 — Verus twin of `compact_through_unleft`

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0108](0108-verus-pending-joint-node.md), [0100](0100-compact-unleft-joint.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. RFC-0100 production `compact_through_unleft` caps compact at `joint-1` until leave. The Verus twin `compact_kernel.rs` only proved `may_compact_through` (F27/F28). Freeze would still pass if the cap vanished from the twin. AS-IS returns `through`. This slice puts the cap in the twin and catalog. A Verus twin is not a verified verifier (R-verus). 0100 RAM compact and 0101 reopen are **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- RFC-0100/0108 P1.1 named this leftover.
- Catalog `compact` entry is `may_compact_through`. Production `maybe_compact_logs` calls `compact_through_unleft`.

## Problems This Solves

- **Problem:** un-left compact cap existed in production, not in the Verus twin.
- **Problem:** freeze would still pass if `compact_through_unleft` vanished from the twin.
- **Problem:** AS-IS compact-past-joint had no twin lemma.

## Proposed Solution

- Twin exec `compact_through_unleft` / `_as_is`. Catalog pair `compact_unleft`. Named rust glue tooth `unleft_joint_caps_through`. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (twin + freeze pair)
- [x] **P0.1** Twin + catalog pair `compact_unleft` — status: `done`
- [x] **P0.2** Regression: AS-IS compact past joint — status: `done` (`unleft_joint_caps_through`)

### P1 — next wave
- [x] **P1.1** Verus twin of `joint_leave_ok` — status: `done` (RFC-0110)
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [ ] **P2.2** R-verus still never — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | twin + catalog compact_unleft | done | compact_kernel.rs + catalog.json | 2026-08-28 |
| P0.2 | p0 | AS-IS compact past un-left joint | done | unleft_joint_caps_through | 2026-08-28 |
| P1.1 | p1 | joint_leave_ok twin | done | RFC-0110 | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | R-verus never | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `unleft_joint_caps_through`: `compact_through_unleft(5, Some(3)) == 2`; AS-IS `== 5`.
  - Catalog pair `compact_unleft` `entry: compact_through_unleft`; freeze twins fail if the exec fn is dropped from the twin.
  - `./scripts/aeneas_store_compact.sh` — Charon+Aeneas extract of the rustc bodies (single artifact; the `verus/compact_kernel.rs` twin was deleted 2026-09-09).
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0108 P1.1 split (`joint_leave_ok` stays); `residuals.json` `R-joint` owner 0109.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
