# RFC: 0102 — After compact+reopen, C-old majority still must not elect

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0101](0101-unleft-joint-compact-reopen.md), [0066](0066-joint-leave-fail-closed.md), [0068](0068-world-plant-committed-joint-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig** + **G1 process death**. RFC-0101 keeps the un-left joint **bytes** on the reopened log. AS-IS compact+reopen drops C-old,new; `pending_joint_on` is None and `election_has_joint_quorum` elects on C-old only (0066 hole after crash). This slice names the **election** gate on that durable path. 0101 log-only and 0066/0068 RAM plant are **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `election_has_joint_quorum` reads `pending_joint()` (live logs), not cluster `ids` alone.
- `crash_reopen_engine_on` reloads log via `load_range_peer` (`index > snap`).
- 0066/0068 refuse old-only **before** compact/reopen. 0101 does not call the election kernel.

## Problems This Solves

- **Problem:** joint bytes on disk can still be ignored at the next election.
- **Problem:** AS-IS `joint_election_ok` ignores C-new once `pending_joint` is empty.
- **Problem:** 0101 is not an election tooth.

## Proposed Solution

- Same compact cap. Named test: 0101 persist+compact+reopen, then `election_has_joint_quorum` with a C-old majority is false. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (election after reopen)
- [x] **P0.1** Reopened un-left joint still gates election — status: `done`
- [x] **P0.2** Regression — status: `done` (`election_after_compact_reopen_refuses_old_majority`)

### P1 — next wave
- [ ] **P1.1** Verus twin of `compact_through_unleft` (0100 P1.1) — status: `todo`
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [x] **P2.2** Queued add then remove round-trip — status: `done` (RFC-0103)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | reopen joint gates election | done | election_has_joint_quorum after crash_reopen | 2026-08-28 |
| P0.2 | p0 | old majority refused after reopen | done | election_after_compact_reopen_refuses_old_majority | 2026-08-28 |
| P1.1 | p1 | Verus twin | todo | — | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | Queued add then remove | done | RFC-0103 | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_election_ok(1, 1, Some((1, 2)))` false; AS-IS true.
  - `election_after_compact_reopen_refuses_old_majority`: Direct elect 2, shrink, plant, persist, compact, `crash_reopen_engine_on`. C-old majority (`election_has_joint_quorum`) is false. 0101 log-only and 0066/0068 RAM plant are **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0101 P1 stays Verus/`cluster_real`; `residuals.json` `R-joint` owner 0102.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
