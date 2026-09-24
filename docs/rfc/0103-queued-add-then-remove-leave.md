# RFC: 0103 — Queued add then remove must leave both joints

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0102](0102-election-after-compact-reopen.md), [0098](0098-queued-add-member-leave.md), [0099](0099-queued-remove-member-leave.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig** + **G4 Queued**. RFC-0098 leaves after Queued **add**; RFC-0099 leaves after Queued **remove** from a Direct-elected 4. Neither is add-then-remove on the same Queued pin: a second joint can start while the add leave is uncommitted (`pending_joint_on` still sees C-old,new) or skip leave on the shrink. AS-IS `joint_leave_ok` skips leave. This slice names the round-trip: after Queued add **and** Queued remove, each side produced C-new-only. 0098/0099 single-op teeth are **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Production RPC is Queued. `add_member_joint` / `remove_member_joint` return `NotCommitted` before `leave_joint`; `finish_queued_propose` leaves before compact.
- 0098 extra pump keeps add-leave because the joining node's applied lags. 0099 must observe shrink-leave **before** the remaining 3 compact.
- 0100 caps compact past an un-left joint; leave still must be appended.

## Problems This Solves

- **Problem:** add-leave then shrink was untested under one Queued pin.
- **Problem:** `remove_member_joint` refuses if add leave is still uncommitted (joint in flight) — round-trip must drain leave first.
- **Problem:** AS-IS still treats leave as optional.

## Proposed Solution

- Same leave path. Named Queued add-then-remove test. Drain add leave (`pending_joint` None) before shrink. Observe shrink leave before compact. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (Queued add then remove leaves)
- [x] **P0.1** Queued add then remove both leave — status: `done`
- [x] **P0.2** Regression — status: `done` (`leave_joint_on_queued_add_then_remove_is_in_log`)

### P1 — next wave
- [ ] **P1.1** Verus twin of `compact_through_unleft` / `joint_leave_ok` — status: `todo`
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [x] **P2.2** Queued add-remove-add round-trip — status: `done` (RFC-0104)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Queued add then remove leaves | done | add_member_joint + remove_member_joint | 2026-08-28 |
| P0.2 | p0 | round-trip leave in log | done | leave_joint_on_queued_add_then_remove_is_in_log | 2026-08-28 |
| P1.1 | p1 | Verus twin | todo | — | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | add-remove-add Queued | done | RFC-0104 | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_leave_ok(false)` false; AS-IS true.
  - `leave_joint_on_queued_add_then_remove_is_in_log`: Direct elect 4, Direct shrink 4, pin Queued, `add_member_joint(4)` pump+finish (leave observed), drain until `pending_joint` None and member 4 is in, then `remove_member_joint(4)` pump+finish (shrink leave observed before compact). 0098 add-only and 0099 Direct-4 remove are **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0102 P2.2 done; `residuals.json` `R-joint` owner 0103.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
