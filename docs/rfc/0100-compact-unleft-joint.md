# RFC: 0100 — Compact must not drop an un-left still-active joint

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0099](0099-queued-remove-member-leave.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. RFC-0096–0099 observed C-new-only **before** compact: `compact_through(min_applied)` drops an applied `MembershipJoint` even when no leave (`old == new`) has been applied, so `pending_joint_on` goes None and election/leave lose C-old,new. AS-IS `compact_through_unleft` ignores the joint. This slice caps production compact at `joint_index - 1` until a leave is applied. Queued remove leave-in-log (0099) is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `maybe_compact_logs` drops `index <= min(applied)` once every member has applied.
- A still-active joint (`old != new`) that is applied but not left is the Raft §6 in-flight config.
- 0096 had to plant+leave because Direct `add_member_joint` compact hid membership from RAM.
- 0099 observes shrink leave **before** the remaining 3 catch up and compact.

## Problems This Solves

- **Problem:** compact can erase C-old,new before leave, so `pending_joint_on` is None.
- **Problem:** AS-IS compact treats membership like any other prefix.
- **Problem:** 0096–0099 teeth are “observe bytes before compact,” not a production cap.

## Proposed Solution

- Pure `compact_through_unleft(through, unleft_joint)` = `joint-1` when the joint is in `[1, through]`. AS-IS returns `through`. Production `maybe_compact_logs` uses the cap. No new `*_kernel.rs` (existing `compact_kernel.rs`).

## Delivery slices (mandatory)

### P0 — must ship first (compact keeps un-left joint)
- [x] **P0.1** `compact_through_unleft` + AS-IS + `maybe_compact_logs` cap — status: `done`
- [x] **P0.2** Regression — status: `done` (`compact_does_not_drop_unleft_joint`)

### P1 — next wave
- [ ] **P1.1** Verus twin of `compact_through_unleft` — status: `todo`
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [ ] **P2.2** Queued add then remove round-trip (0099 P2.2) — status: `todo`
- compact persist + crash-reopen is RFC-0101 (not this P1)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | compact cap un-left joint | done | compact_kernel + maybe_compact_logs | 2026-08-28 |
| P0.2 | p0 | compact keeps un-left joint | done | compact_does_not_drop_unleft_joint | 2026-08-28 |
| P1.1 | p1 | Verus twin | todo | — | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | Queued add then remove | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `compact_through_unleft(5, Some(3)) == 2`; AS-IS `== 5`.
  - `compact_does_not_drop_unleft_joint`: Direct elect 2, shrink to 1, `plant_committed_joint_without_leave`, close apply lag without leave, `maybe_compact_logs`. Leader log still has `joint_still_active` membership (`pending_joint_on` Some). AS-IS cap would compact through the joint. 0099 leave-before-compact is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0099 P1 stays Verus/`cluster_real`; `residuals.json` `R-joint` owner 0100.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
