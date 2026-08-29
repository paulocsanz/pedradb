# RFC: 0113 — RequestVote must not go to a lagging removed node

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0112](0112-vote-targets-omit-removed.md), [0105](0105-pending-joint-members-only.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. RFC-0112 asserts `vote_targets()` omits 4. Production `start_election` still sends if a pid is in targets **and** not `!participating`. A removed process that has not applied the shrink still has `participating=true` (or is remote, so the local skip does not fire). AS-IS `pending_joint_node_counts` puts 4 back in targets, so RV is queued to the ex-member. This slice names the **wire**: after shrink leave, `start_election` under Queued must not enqueue `RequestVote` to 4 even if that node is still marked participating. 0112 helper-only is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `start_election` walks `vote_targets()` then skips local `!participating`; remotes always send.
- After shrink apply in-process, 4 is `participating=false`, so 0112's omit is masked by the skip.
- The hole is a lagging ex-member that is still “up.”

## Problems This Solves

- **Problem:** RV send to the removed node was untested (helper ≠ outbound).
- **Problem:** participating skip hid 0112 on the in-process path.
- **Problem:** AS-IS would still queue RV to 4.

## Proposed Solution

- Same `pending_joint_node_counts`. Named test: after shrink leave, set 4 participating (lag), `start_election`, drain outbound, no `RequestVote` to 4, some RV to a live peer. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (no RV on the wire to removed)
- [x] **P0.1** `start_election` does not enqueue RV to removed — status: `done`
- [x] **P0.2** Regression — status: `done` (`request_vote_not_sent_to_lagging_removed_node`)

### P1 — next wave
- [ ] **P1.1** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`
- inbound grant from removed node is RFC-0114 (not this P1)
- [ ] **P1.2** Campaign is not ∀ traces — status: `todo`

### P2 — later
- [ ] **P2.1** R-verus still never — status: `todo`
- [ ] **P2.2** none yet

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | no RV to lagging removed | done | start_election outbound | 2026-08-28 |
| P0.2 | p0 | RV not queued to 4 | done | request_vote_not_sent_to_lagging_removed_node | 2026-08-28 |
| P1.1 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P1.2 | p1 | not ∀ traces | todo | — | 2026-08-28 |
| P2.1 | p2 | R-verus never | todo | — | 2026-08-28 |
| P2.2 | p2 | none yet | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `pending_joint_node_counts(false)` false; AS-IS true.
  - `request_vote_not_sent_to_lagging_removed_node`: Queued add+shrink as 0105; node 4 still has `joint_still_active`; force `participating=true` on 4; `start_election` on a live member; drain outbound; no `RequestVote` with `to==4`; at least one RV to a live id. 0112 `vote_targets()` is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0112 P1 stays `cluster_real`; `residuals.json` `R-joint` owner 0113.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
