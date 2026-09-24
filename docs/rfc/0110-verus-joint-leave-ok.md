# RFC: 0110 — Verus twin of `joint_leave_ok`

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0109](0109-verus-compact-through-unleft.md), [0096](0096-live-leave-joint-in-log.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. RFC-0096 production `joint_leave_ok(leave_in_log)` is the bool; AS-IS always true (skip leave). Live leave-in-log tests (0096–0107) call it. The Verus twin had `joint_still_active` (0095) but not this gate — freeze would still pass if it vanished. This slice puts `joint_leave_ok` in the twin and catalog. A Verus twin is not a verified verifier (R-verus). 0096 plant+leave is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- RFC-0096–0109 P1 named this leftover.
- Catalog `joint_leave` entry is `joint_still_active`. Clone already lists `joint_leave_ok`.

## Problems This Solves

- **Problem:** leave-required existed in production, not in the Verus twin.
- **Problem:** freeze would still pass if `joint_leave_ok` vanished from the twin.
- **Problem:** AS-IS skip-leave had no twin lemma.

## Proposed Solution

- Twin exec `joint_leave_ok` / `_as_is` (`leave_in_log` / `true`). Catalog pair `joint_leave_ok`. Named rust glue tooth `joint_leave_ok_requires_leave_in_log`. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (twin + freeze pair)
- [x] **P0.1** Twin + catalog pair `joint_leave_ok` — status: `done`
- [x] **P0.2** Regression: AS-IS skips leave — status: `done` (`joint_leave_ok_requires_leave_in_log`)

### P1 — next wave
- [ ] **P1.1** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`
- [x] **P1.2** Queued four-step (remove-add-remove-add) — status: `done` (RFC-0111)

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [ ] **P2.2** R-verus still never — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | twin + catalog joint_leave_ok | done | membership_joint.rs + catalog.json | 2026-08-28 |
| P0.2 | p0 | AS-IS skips leave | done | joint_leave_ok_requires_leave_in_log | 2026-08-28 |
| P1.1 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P1.2 | p1 | four-step Queued | done | RFC-0111 | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | R-verus never | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_leave_ok_requires_leave_in_log`: `joint_leave_ok(false)` false; AS-IS true. Same tokens raft=store.
  - Catalog pair `joint_leave_ok` `entry: joint_leave_ok`; freeze twins fail if the exec fn is dropped from the twin.
  - `./scripts/verus_membership_joint.sh` when Verus is installed.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0109 P1.1 done as this RFC; `residuals.json` `R-joint` owner 0110.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
