# RFC: 0111 — Queued remove-add-remove-add must leave the last add

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0110](0110-verus-joint-leave-ok.md), [0106](0106-queued-remove-add-remove-leave.md), [0104](0104-queued-add-remove-add-leave.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig** + **G4 Queued**. RFC-0106 is remove-add-remove (stops at the second shrink). RFC-0104 is add-remove-add after a **Direct** shrink. Neither is four steps under **one** Queued pin ending in add after **two** Queued shrinks: the last add can still hit “joint already in flight” if the second shrink leave is uncommitted. AS-IS `joint_leave_ok` skips leave. This slice names that last add. 0104/0106 are **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Production RPC is Queued. After two shrinks, `pending_joint_node_counts` ignores the ex-member (0105) but the **leader** still refuses a new joint while leave is in flight.
- 0106 does not drain the second shrink or re-add.

## Problems This Solves

- **Problem:** second Queued shrink → add was untested.
- **Problem:** 0104 last add is after one Direct + one Queued shrink, not two Queued shrinks.
- **Problem:** AS-IS still treats leave as optional.

## Proposed Solution

- Same leave + drain path. Named four-step test. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (Queued four-step last add leaves)
- [x] **P0.1** Queued remove-add-remove-add last add leaves — status: `done`
- [x] **P0.2** Regression — status: `done` (`leave_joint_on_queued_remove_add_remove_add_is_in_log`)

### P1 — next wave
- [ ] **P1.1** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`
- RV targets after shrink leave is RFC-0112 (not this P1)
- [ ] **P1.2** Campaign is not ∀ traces — status: `todo`

### P2 — later
- [ ] **P2.1** R-verus still never — status: `todo`
- [ ] **P2.2** none yet

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | four-step last add leaves | done | add after second Queued shrink | 2026-08-28 |
| P0.2 | p0 | four-step leave in log | done | leave_joint_on_queued_remove_add_remove_add_is_in_log | 2026-08-28 |
| P1.1 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P1.2 | p1 | not ∀ traces | todo | — | 2026-08-28 |
| P2.1 | p2 | R-verus never | todo | — | 2026-08-28 |
| P2.2 | p2 | none yet | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_leave_ok(false)` false; AS-IS true.
  - `leave_joint_on_queued_remove_add_remove_add_is_in_log`: Direct elect 4, pin Queued, remove 4 leave+drain, add 4 leave+drain, remove 4 leave+drain, add 4 admitted (not “already in flight”) and leave observed. 0106 three-step and 0104 Direct-shrink add-remove-add are **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0110 P1.2 done; `residuals.json` `R-joint` owner 0111.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
