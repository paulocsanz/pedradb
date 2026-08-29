# RFC: 0114 — RequestVote grant from a non-voter must not be recorded

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0113](0113-no-rv-to-removed-node.md), [0105](0105-pending-joint-members-only.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. RFC-0113 stops **sending** RV to a lagging removed node. `on_request_vote_reply` still pushed `from` into `election_granted` with no membership check. A stale/lagging grant from node 4 is recorded. During an in-flight joint, C-new (add) or C-old (shrink) voters must still count. AS-IS `election_grant_from_counts` always true. This slice names the gate: grant counts iff `ids` **or** pending old∪new. 0113 send-side is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `on_request_vote_reply` recorded every `vote_granted`.
- After leave, `pending_joint` is None and 4 is not in `ids`; a grant from 4 must not land in `election_granted`.
- During joint add, 4 is in C-new but not yet in `ids` — that grant **must** still count.

## Problems This Solves

- **Problem:** lagging removed voter was still recorded.
- **Problem:** 0113 only covered outbound RV.
- **Problem:** AS-IS records any `from`.

## Proposed Solution

- Pure `election_grant_from_counts(in_ids, in_pending_old_or_new)` = or. AS-IS always true. Production reply path uses it. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (grant only from voters)
- [x] **P0.1** `election_grant_from_counts` + reply gate — status: `done`
- [x] **P0.2** Regression — status: `done` (`rv_grant_from_removed_node_is_ignored_after_leave`)

### P1 — next wave
- [ ] **P1.1** Verus twin of `election_grant_from_counts` — status: `todo`
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [x] **P2.2** joint-add grant from joining node still counts — status: `done` (RFC-0115)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | grant-from kernel + reply | done | membership_kernel + on_request_vote_reply | 2026-08-28 |
| P0.2 | p0 | removed grant ignored after leave | done | rv_grant_from_removed_node_is_ignored_after_leave | 2026-08-28 |
| P1.1 | p1 | Verus twin | todo | — | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | joint-add joining vote | done | RFC-0115 | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `election_grant_from_counts(false, false)` false; AS-IS true; `(false, true)` and `(true, false)` true.
  - `rv_grant_from_removed_node_is_ignored_after_leave`: Queued add+shrink as 0105; `start_election`; `on_request_vote_reply` from 4 granted; `election_granted` does not contain 4. 0113 outbound is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0113 P1 stays `cluster_real`; `residuals.json` `R-joint` owner 0114.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
