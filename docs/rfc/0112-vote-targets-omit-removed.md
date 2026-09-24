# RFC: 0112 — After shrink leave, RequestVote targets must omit the removed node

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0111](0111-queued-remove-add-remove-add-leave.md), [0107](0107-election-ignores-removed-joint.md), [0105](0105-pending-joint-members-only.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. `start_election` / `try_become_leader` send RV to `vote_targets()`, which is `ids` plus `pending_joint` old∧new. RFC-0105 skips removed nodes in `pending_joint`; 0107 is **quorum** (2/3 elects). AS-IS `pending_joint_node_counts` still sees the ex-member's C-old,new, so `vote_targets` would still list node 4 and a multi-host send would RV a non-voter. This slice names the **target set**. 0105/0107 are **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Production election walks `vote_targets()` then skips `!participating` locals; remotes still get the RPC.
- After Queued shrink leave, ids = C-new; the removed node's log can still hold C-old,new.

## Problems This Solves

- **Problem:** RV target set was untested after shrink leave.
- **Problem:** 0107 does not read `vote_targets`.
- **Problem:** AS-IS would still target the ex-member.

## Proposed Solution

- Same `pending_joint_node_counts`. Named test: after shrink leave, `vote_targets()` equals live `ids` and omits 4 while node 4's log still has `joint_still_active`. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (RV targets omit removed)
- [x] **P0.1** `vote_targets` after shrink leave omits removed — status: `done`
- [x] **P0.2** Regression — status: `done` (`vote_targets_after_queued_shrink_omit_removed_node`)

### P1 — next wave
- [ ] **P1.1** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`
- RV on the wire to a lagging removed node is RFC-0113 (not this P1)
- [ ] **P1.2** Campaign is not ∀ traces — status: `todo`

### P2 — later
- [ ] **P2.1** R-verus still never — status: `todo`
- [ ] **P2.2** none yet

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RV targets omit removed | done | vote_targets after shrink leave | 2026-08-28 |
| P0.2 | p0 | vote_targets omit 4 | done | vote_targets_after_queued_shrink_omit_removed_node | 2026-08-28 |
| P1.1 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P1.2 | p1 | not ∀ traces | todo | — | 2026-08-28 |
| P2.1 | p2 | R-verus never | todo | — | 2026-08-28 |
| P2.2 | p2 | none yet | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `pending_joint_node_counts(false)` false; AS-IS true.
  - `vote_targets_after_queued_shrink_omit_removed_node`: same Queued add+shrink as 0105; node 4 still has `joint_still_active`; `vote_targets()` equals live ids and does not contain 4. 0107 quorum and 0105 pending-None are **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0111 P1 stays `cluster_real`; `residuals.json` `R-joint` owner 0112.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
