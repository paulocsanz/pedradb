# RFC: 0104 — After Queued shrink leave, Queued add must be admitted and leave

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0103](0103-queued-add-then-remove-leave.md), [0098](0098-queued-add-member-leave.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig** + **G4 Queued**. RFC-0103 leaves add then remove. After shrink, `pending_joint` can stick (leave uncommitted / 0100 keeps un-left joint) so the next `add_member_joint` hits “joint already in flight.” 0098 add is after **Direct** shrink, not after Queued shrink. AS-IS `joint_leave_ok` skips leave. This slice names the gate: drain shrink leave, then Queued add is admitted and produces C-new-only. 0103 add-then-remove is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `add_member_joint` / `remove_member_joint` refuse if `pending_joint_on` is Some.
- 0100 compact keeps an un-left joint, so a stuck shrink joint blocks the next add.
- 0103 stops at remove leave; does not re-add.

## Problems This Solves

- **Problem:** Queued shrink → add was untested; joint-in-flight can pin membership.
- **Problem:** 0098 add is not after a Queued shrink.
- **Problem:** AS-IS still treats leave as optional.

## Proposed Solution

- Drain shrink leave (`pending_joint` None, node not a member) then Queued add and observe leave. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (Queued add after Queued shrink)
- [x] **P0.1** Queued add after shrink is admitted and leaves — status: `done`
- [x] **P0.2** Regression — status: `done` (`leave_joint_on_queued_add_remove_add_is_in_log`)

### P1 — next wave
- [ ] **P1.1** Verus twin of `compact_through_unleft` / `joint_leave_ok` — status: `todo`
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [ ] **P2.2** Queued remove-add-remove — status: `todo`
- cluster-wide pending_joint after shrink is RFC-0105 (not this P1)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | add after Queued shrink leaves | done | add_member_joint after remove drain | 2026-08-28 |
| P0.2 | p0 | add-remove-add leave in log | done | leave_joint_on_queued_add_remove_add_is_in_log | 2026-08-28 |
| P1.1 | p1 | Verus twin | todo | — | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | Queued remove-add-remove | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_leave_ok(false)` false; AS-IS true.
  - `leave_joint_on_queued_add_remove_add_is_in_log`: Direct elect 4, Direct shrink 4, pin Queued, add 4 (leave), drain, remove 4 (leave), drain until not a member, second `add_member_joint(4)` is admitted (not “joint already in flight”) and leave is observed. Cluster-wide `pending_joint` on a lagging follower is **not** this tooth (`add_member_joint` reads the leader). 0103 add-then-remove and 0098 Direct-shrink add are **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0103 P2.2 done; `residuals.json` `R-joint` owner 0104.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
