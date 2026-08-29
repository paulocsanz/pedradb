# RFC: 0097 — Queued RPC `leave_joint` still writes C-new-only (production mode, not Direct lab)

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0096](0096-live-leave-joint-in-log.md), [0067](0067-dst-queued-rpc-pin-fail-closed.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig** plus **G4 Direct bypass**. Production `StoreCluster::open` is [`RpcMode::Queued`](0067). RFC-0096’s leave tooth ran under `open_lab_direct`. AS-IS `joint_leave_ok` still skips leave. This slice names the gate on the **Queued** path: after a planted committed joint, `leave_joint` writes `old == new`. Direct-lab 0096 is **not** this tooth. `cluster_real` 3-process leave is still 0066 P2.2.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- RFC-0067: World fingerprint is Queued; Direct is lab opt-in.
- RFC-0096: `leave_joint` after plant writes leave **in Direct**.
- Compacted `add_member_joint` logs still are not this tooth.

## Problems This Solves

- **Problem:** leave was only shown under Direct pump.
- **Problem:** production RPC is Queued; Direct can hide Net-shaped delivery.
- **Problem:** AS-IS still treats leave as optional.

## Proposed Solution

- Same `joint_leave_ok`. Named test: plant under Direct, pin Queued, `leave_joint` + pump, leader log has C-new-only. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (Queued leave)
- [x] **P0.1** Queued `leave_joint` writes leave — status: `done`
- [x] **P0.2** Regression — status: `done` (`leave_joint_on_queued_store_is_in_log`)

### P1 — next wave
- [ ] **P1.1** Verus twin of `joint_leave_ok` — status: `todo`
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [x] **P2.2** Queued `add_member_joint` (not only plant+leave) — status: `done` (RFC-0098)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Queued leave writes | done | leave_joint + pin_dst_queued | 2026-08-28 |
| P0.2 | p0 | Queued leave in log | done | leave_joint_on_queued_store_is_in_log | 2026-08-28 |
| P1.1 | p1 | Verus twin | todo | — | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | Queued add_member | done | RFC-0098 | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_leave_ok(false)` false; AS-IS true.
  - `leave_joint_on_queued_store_is_in_log`: plant committed joint, `pin_dst_queued`, `leave_joint`, pump. `rpc_mode` is Queued. Leader log has `!joint_still_active` membership. Direct 0096 test is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0096 P2.2 done; `residuals.json` `R-joint` owner 0097.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
