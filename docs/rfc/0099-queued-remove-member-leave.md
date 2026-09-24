# RFC: 0099 — Queued `remove_member_joint` must leave after the joint commits

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0098](0098-queued-add-member-leave.md), [0097](0097-queued-leave-joint.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig** + **G4 Queued**. RFC-0098 fixed Queued **add**: `NotCommitted` skipped `leave_joint`; `finish_queued_propose` now leaves before compact. **Remove** uses the same append+leave pair. AS-IS `joint_leave_ok` skips leave. This slice names the gate on live Queued **remove**: after pump+finish, the leader log has C-new-only. Queued add (0098) is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `remove_member_joint` does `broadcast_append(joint)` then `leave_joint_after_commit`.
- 0098’s named test only covers add. Direct remove compact is not this tooth.

## Problems This Solves

- **Problem:** leave on shrink was untested under Queued.
- **Problem:** AS-IS still treats leave as optional.
- **Problem:** add≠remove (C-new smaller; joint quorum uses the leaving set).

## Proposed Solution

- Same `leave_joint_after_commit` path. Named Queued remove test. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (Queued remove leaves)
- [x] **P0.1** Queued `remove_member_joint` + finish leaves — status: `done`
- [x] **P0.2** Regression — status: `done` (`leave_joint_on_queued_remove_member_is_in_log`; observe leave before compact drops C-new-only)

### P1 — next wave
- [ ] **P1.1** Verus twin of `joint_leave_ok` — status: `todo`
- [ ] **P1.2** L28 REAL leave on `cluster_real` — status: `todo`
- compact-before-leave is RFC-0100 (not this P1)

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [ ] **P2.2** Queued add then remove round-trip — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Queued remove leaves | done | remove_member_joint + finish | 2026-08-28 |
| P0.2 | p0 | Queued remove leave in log | done | leave_joint_on_queued_remove_member_is_in_log | 2026-08-28 |
| P1.1 | p1 | Verus twin | todo | — | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | add then remove Queued | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_leave_ok(false)` false; AS-IS true.
  - `leave_joint_on_queued_remove_member_is_in_log`: Direct elect 4, pin Queued, `remove_member_joint(4)`, pump+finish. A node log has `!joint_still_active` membership **before** compact-after-apply drops it (remaining 3 catch up; 0098 add is **not** this tooth — joining node's applied lags). Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0098 P2.2 done; `residuals.json` `R-joint` owner 0099.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
