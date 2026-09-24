# RFC: 0101 — Un-left joint must survive compact persist + crash-reopen

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0100](0100-compact-unleft-joint.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig** + **G1 process death**. RFC-0100 caps RAM compact at `joint-1` (`compact_through_unleft`). AS-IS compact persists a snapshot **through** the joint; `load_range_peer` then drops `index <= snapshot`, so reopen has no C-old,new. This slice names the gate on the **durable** path: after `maybe_compact_logs` + `crash_reopen_engine_on`, the leader log still has a still-active joint. 0100 RAM-only compact is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `load_range_peer` retains `index > snapshot_index` and `index <= commit`.
- Compact persist writes snap + log. AS-IS `compact_through_unleft` lets snap pass the joint.
- Plant is RAM (0068 DST seam); production persist is `persist_log_db` (same fn compact uses).

## Problems This Solves

- **Problem:** 0100 only watched RAM; disk could still forget C-old,new across reopen.
- **Problem:** AS-IS compact+reopen elects on C-old only (`pending_joint_on` None).
- **Problem:** Sim2 process death is simulated; this is a live Pedra reopen.

## Proposed Solution

- Same `compact_through_unleft` cap. Named compact+persist+`crash_reopen_engine_on` test. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (joint on disk after compact+reopen)
- [x] **P0.1** Compact persist keeps un-left joint across reopen — status: `done`
- [x] **P0.2** Regression — status: `done` (`unleft_joint_survives_compact_reopen`)

### P1 — next wave
- [ ] **P1.1** Verus twin of `compact_through_unleft` (0100 P1.1) — status: `todo`
- [x] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `done` (RFC-0121 P1.2)

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [ ] **P2.2** Queued add then remove round-trip — status: `todo`
- election after compact+reopen is RFC-0102 (not this P1)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | compact persist + reopen keeps joint | done | persist_log_db + crash_reopen | 2026-08-28 |
| P0.2 | p0 | un-left joint survives reopen | done | unleft_joint_survives_compact_reopen | 2026-08-28 |
| P1.1 | p1 | Verus twin | todo | — | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | done | RFC-0121 P1.2 l28_real_tcp_remove_member_left_on_disk | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | Queued add then remove | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `compact_through_unleft(5, Some(3)) == 2`; AS-IS `== 5`.
  - `unleft_joint_survives_compact_reopen`: Direct elect 2, shrink to 1, `plant_committed_joint_without_leave`, persist log/commit/applied via production `persist_*_db`, close apply lag, `maybe_compact_logs`, `crash_reopen_engine_on` the leader. Reopened log has `joint_still_active` membership. 0100 RAM compact is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0100 P1 stays Verus/`cluster_real`; `residuals.json` `R-joint` owner 0101.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
