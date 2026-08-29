# RFC: 0107 — After shrink leave, remaining majority must elect despite a zombie joint

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0105](0105-pending-joint-members-only.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. RFC-0105 made `pending_joint()` skip removed nodes. `election_has_joint_quorum` reads that map. AS-IS `pending_joint_node_counts` still scans the ex-member: leftover C-old,new (`old_n=4`) means 2/3 of the live set is **not** a joint quorum (`joint_election_ok(2,4,Some((2,3)))` false). Production after leave must elect on C-new only (`ids`, `new=None`). 0105 is `pending_joint()` None; 0102 is compact+reopen plant. Neither is this election tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- After Queued shrink, the removed node keeps C-old,new (leave AE did not target it).
- Election uses `pending_joint()` → `joint_election_ok`.
- 2 of remaining 3 is a majority of C-new=3; it is not majority of C-old=4.

## Problems This Solves

- **Problem:** zombie joint on the ex-member can block a valid live-set election.
- **Problem:** 0105 did not call `election_has_joint_quorum`.
- **Problem:** AS-IS counts the removed node.

## Proposed Solution

- Same `pending_joint_node_counts`. Named test: after shrink leave, 2/3 live grants elect; AS-IS 2/4 old would not. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (live-set election after shrink)
- [x] **P0.1** Remaining majority elects after shrink leave — status: `done`
- [x] **P0.2** Regression — status: `done` (`election_after_queued_shrink_ignores_removed_node_joint`)

### P1 — next wave
- [x] **P1.1** Verus twin of `pending_joint_node_counts` — status: `done` (RFC-0108)
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [ ] **P2.2** Queued four-step add-remove-add-remove (0106 P2.2) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | live-set elects after shrink | done | election_has_joint_quorum | 2026-08-28 |
| P0.2 | p0 | 2/3 not blocked by zombie joint | done | election_after_queued_shrink_ignores_removed_node_joint | 2026-08-28 |
| P1.1 | p1 | Verus twin | done | RFC-0108 | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | four-step Queued | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_election_ok(2, 3, None)` true; `joint_election_ok(2, 4, Some((2, 3)))` false; AS-IS `pending_joint_node_counts_as_is(false)` true.
  - `election_after_queued_shrink_ignores_removed_node_joint`: same Queued add+shrink as 0105; node 4 still has `joint_still_active`; `pending_joint()` None; 2 live grants → `election_has_joint_quorum` true. 0105 pending-None and 0102 compact-reopen are **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0105 P1 stays Verus/`cluster_real`; `residuals.json` `R-joint` owner 0107.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
