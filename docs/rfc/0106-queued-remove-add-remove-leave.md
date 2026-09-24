# RFC: 0106 — Queued remove then add then remove must leave each joint

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0105](0105-pending-joint-members-only.md), [0104](0104-queued-add-remove-add-leave.md), [0099](0099-queued-remove-member-leave.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig** + **G4 Queued**. RFC-0099 is a single Queued shrink from a Direct-elected 4. RFC-0104 is add-remove-add after a **Direct** shrink. Neither is remove-add-remove under **one** Queued pin: the first joint is a Queued shrink (C-new smaller; 0105 members-only `pending_joint`). AS-IS `joint_leave_ok` skips leave. This slice names the round-trip. 0099/0104 are **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Production RPC is Queued. Shrink leave must clear leader joint so add is admitted (0104) and `pending_joint` ignores the ex-member (0105).
- 0099 observes one shrink leave. 0104 starts after Direct `remove_member_joint`.

## Problems This Solves

- **Problem:** Queued shrink → add → shrink was untested.
- **Problem:** first joint is shrink (quorum old∧new with leaving set), not the 0104 expand.
- **Problem:** AS-IS still treats leave as optional.

## Proposed Solution

- Same leave + drain path. Named Queued remove-add-remove test. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (Queued remove-add-remove leaves)
- [x] **P0.1** Queued remove then add then remove all leave — status: `done`
- [x] **P0.2** Regression — status: `done` (`leave_joint_on_queued_remove_add_remove_is_in_log`)

### P1 — next wave
- [ ] **P1.1** Verus twin of `pending_joint_node_counts` / `compact_through_unleft` — status: `todo`
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [ ] **P2.2** Queued four-step add-remove-add-remove — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Queued remove-add-remove leaves | done | remove + add + remove | 2026-08-28 |
| P0.2 | p0 | round-trip leave in log | done | leave_joint_on_queued_remove_add_remove_is_in_log | 2026-08-28 |
| P1.1 | p1 | Verus twin | todo | — | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | four-step Queued | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_leave_ok(false)` false; AS-IS true.
  - `leave_joint_on_queued_remove_add_remove_is_in_log`: Direct elect 4, pin Queued (no Direct shrink), `remove_member_joint(4)` leave+drain, `add_member_joint(4)` admitted and leave+drain, `remove_member_joint(4)` leave. New leave index each op. 0099 single shrink and 0104 Direct-shrink add-remove-add are **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0105 P2.2 done; `residuals.json` `R-joint` owner 0106.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
