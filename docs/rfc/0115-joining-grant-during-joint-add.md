# RFC: 0115 — RequestVote grant from a joining node counts during joint add

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0114](0114-election-grant-from-member.md), [0068](0068-world-plant-committed-joint-fail-closed.md), [0064](0064-joint-election-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. RFC-0114 records a grant only if `ids` **or** pending old∪new. After leave, 4 is neither. During **joint add**, 4 is in C-new but **not** yet in `ids` (apply lag / plant). A gate that only checked `ids` would drop the joining vote (Raft §6 C-new). AS-IS always records. This slice names the live path: plant committed add-joint without leave, grant from 4 is in `election_granted`. 0114 after-leave ignore is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `plant_committed_joint_without_leave` (0068) puts C-old,new on the leader with apply lag; `ids` stays C-old.
- `election_grant_from_counts(false, true)` is the joint-add case.

## Problems This Solves

- **Problem:** 0114 ids-only would starve C-new during add.
- **Problem:** no live grant from the joining node.
- **Problem:** AS-IS hides whether pending-new is required.

## Proposed Solution

- Same kernel. Named plant + `on_request_vote_reply` from 4. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (joining grant during add)
- [x] **P0.1** Joining-node grant recorded during joint add — status: `done`
- [x] **P0.2** Regression — status: `done` (`rv_grant_from_joining_node_counts_during_joint_add`)

### P1 — next wave
- [x] **P1.1** Verus twin of `election_grant_from_counts` (0114 P1.1) — status: `done` (RFC-0116)
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [ ] **P2.2** R-verus still never — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | joining grant during add | done | on_request_vote_reply + plant | 2026-08-28 |
| P0.2 | p0 | C-new voter recorded | done | rv_grant_from_joining_node_counts_during_joint_add | 2026-08-28 |
| P1.1 | p1 | Verus twin | done | RFC-0116 | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | R-verus never | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `election_grant_from_counts(false, true)` true; `(false, false)` false; AS-IS true on `(false, false)`.
  - `rv_grant_from_joining_node_counts_during_joint_add`: Direct elect 4, shrink 4, `plant_committed_joint_without_leave(4)`, 4 not in `ids`, pending joint active, pin Queued, `start_election`, `on_request_vote_reply` from 4 granted → `election_granted` contains 4. 0114 after-leave ignore is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0114 P2.2 done; `residuals.json` `R-joint` owner 0115.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
