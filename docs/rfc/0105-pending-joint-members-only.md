# RFC: 0105 — `pending_joint` must ignore removed nodes

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0104](0104-queued-add-remove-add-leave.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. `election_has_joint_quorum` reads `pending_joint()`, which walked **every** `StoreCluster::nodes` entry. After Queued shrink, the removed node still holds C-old,new (leave was broadcast only to `ids`). AS-IS `pending_joint_node_counts` counts non-members, so election keeps requiring the shrink joint after leave on the live set. RFC-0104 admitted the next add on the **leader** only and left cluster-wide `pending_joint` out of scope. This slice names the gate: after shrink leave, `pending_joint()` is None even if the ex-member log still has C-old,new. 0104 leader-add is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `pending_joint()` fed election and `vote_targets`.
- Shrink apply drops the node from `ids`; leave AE uses the new `ids`.
- The removed process object stays in `nodes` with the shrink joint.

## Problems This Solves

- **Problem:** a removed node's log kept joint election alive after live members left.
- **Problem:** AS-IS scans everyone, including `participating=false`.
- **Problem:** 0104 is leader-only add admit.

## Proposed Solution

- Pure `pending_joint_node_counts(is_member)` = the bool. AS-IS always true. Production `pending_joint` skips non-members. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (members-only pending joint)
- [x] **P0.1** `pending_joint_node_counts` + `pending_joint` skip — status: `done`
- [x] **P0.2** Regression — status: `done` (`pending_joint_ignores_removed_node_after_queued_shrink`)

### P1 — next wave
- [ ] **P1.1** Verus twin of `pending_joint_node_counts` / `compact_through_unleft` — status: `todo`
- election after shrink despite zombie joint is RFC-0107 (not this P1)
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [x] **P2.2** Queued remove-add-remove (0104 P2.2) — status: `done` (RFC-0106)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | members-only pending_joint | done | pending_joint_node_counts + pending_joint | 2026-08-28 |
| P0.2 | p0 | removed node does not keep joint | done | pending_joint_ignores_removed_node_after_queued_shrink | 2026-08-28 |
| P1.1 | p1 | Verus twin | todo | — | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | Queued remove-add-remove | done | RFC-0106 | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `pending_joint_node_counts(false)` false; AS-IS true.
  - `pending_joint_ignores_removed_node_after_queued_shrink`: Direct elect 4, Direct shrink 4, pin Queued, add 4, drain, remove 4, drain. Node 4's log still has `joint_still_active`; `pending_joint()` is None. 0104 leader-add is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0104 notes cluster-wide pending; `residuals.json` `R-joint` owner 0105.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
