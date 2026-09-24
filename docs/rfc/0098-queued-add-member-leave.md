# RFC: 0098 — Queued `add_member_joint` must leave after the joint commits

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0097](0097-queued-leave-joint.md), [0096](0096-live-leave-joint-in-log.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig** + **G4 Queued**. `add_member_joint` does `broadcast_append(joint)?; leave_joint()`. Under Queued, the append returns `NotCommitted` and **never calls leave**. `finish_queued_propose` applied and compacted without `leave_joint`. AS-IS `joint_leave_ok` skips leave. This slice calls `leave_joint` on the Queued finish path **before** compact. Plant+leave (0096/0097) is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Production RPC is Queued. Direct `add_member_joint` compact drops membership from RAM (0096).
- `try_advance_commit` already ends in `leave_joint`; `finish_queued_propose` did not.

## Problems This Solves

- **Problem:** Queued add returned before leave.
- **Problem:** compact after finish could hide an active joint from `pending_joint_on`.
- **Problem:** 0097 only tested plant+`leave_joint`, not `add_member_joint`.

## Proposed Solution

- `finish_queued_propose` (commit ≥ index) calls `leave_joint` before `maybe_compact_logs`. Named Queued add test. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (Queued add leaves)
- [x] **P0.1** `finish_queued_propose` calls `leave_joint` before compact — status: `done`
- [x] **P0.2** Regression — status: `done` (`leave_joint_on_queued_add_member_is_in_log`)

### P1 — next wave
- [ ] **P1.1** Verus twin of `joint_leave_ok` — status: `todo`
- [x] **P1.2** L28 REAL leave on `cluster_real` — status: `done` (RFC-0121 P1.2)

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [x] **P2.2** Queued `remove_member_joint` leave — status: `done` (RFC-0099)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | finish_queued_propose leaves | done | lib.rs finish_queued_propose | 2026-08-28 |
| P0.2 | p0 | Queued add leave in log | done | leave_joint_on_queued_add_member_is_in_log | 2026-08-28 |
| P1.1 | p1 | Verus twin | todo | — | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | done | RFC-0121 P1.2 l28_real_tcp_remove_member_left_on_disk | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | Queued remove leave | done | RFC-0099 | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_leave_ok(false)` false; AS-IS true.
  - `leave_joint_on_queued_add_member_is_in_log`: Direct elect/remove, pin Queued, `add_member_joint`, pump+`finish_queued_propose`. Leader log has `!joint_still_active` membership. 0096/0097 plant+leave are **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0097 P2.2 done; `residuals.json` `R-joint` owner 0098.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
