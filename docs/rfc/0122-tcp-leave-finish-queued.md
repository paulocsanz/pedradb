# RFC: 0122 — TCP leave after a joint must finish the queued propose

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0121](0121-tcp-joint-finish-queued.md), [0098](0098-queued-add-member-leave.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` + `R-swarm-real` (not `never_floor`). Axis vs FDB Sim2: **G4 real TCP + G7 reconfig**. RFC-0121 finishes the **joint** propose (`finish_queued_propose` on the C-old,new index). That path calls `leave_joint_after_commit`, which **swallows** `NotCommitted` for the C-new-only entry. The leave sits on the leader log with `index > commit`. `leave_joint` then sees `leave_in_flight` and returns Ok without finishing. AS-IS `queued_leave_finish_ok` treats “in the log” as enough. This slice: production `finish_uncommitted_leave`; TCP worker drives it like put. A 3-process on-disk proof is **not** this tooth (0066 P2.2 / 0121 P1.2).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `finish_queued_propose` after a joint commits appends leave and returns Ok even if that leave is `NotCommitted`.
- `leave_joint` no-ops when an uncommitted leave is already in the log.
- In-process tests pump extra rounds; `montanha-tcp` did not.

## Problems This Solves

- **Problem:** TCP plant commits the joint and leaves C-new-only uncommitted.
- **Problem:** CLI `leave-joint` no-ops on in-flight leave.
- **Problem:** AS-IS “in the log” skips commit.

## Proposed Solution

- Pure `queued_leave_finish_ok(leave_in_log, leave_committed)` = `!leave_in_log || leave_committed`. AS-IS = `leave_in_log`. `StoreCluster::finish_uncommitted_leave`. TCP worker drives it after membership finish and on `LeaveJoint`. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (finish the leave propose)
- [x] **P0.1** `queued_leave_finish_ok` + `finish_uncommitted_leave` + TCP worker — status: `done`
- [x] **P0.2** Regression — status: `done` (`queued_leave_after_joint_must_be_finished`)

### P1 — next wave
- [x] **P1.1** `LeaveJoint` worker uses the same finish path — status: `done`
- [x] **P1.2** 3-process C-new-only on disk (0066 P2.2) — status: `done` (`l28_real_tcp_remove_member_left_on_disk`)

### P2 — later
- [x] **P2.1** Verus twin of `queued_leave_finish_ok` — status: `done` (RFC-0123)
- [x] **P2.2** Campaign is not ∀ traces — status: `done` (`queued_leave_finish_campaign_is_not_forall_traces`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | finish uncommitted leave | done | membership_kernel + lib.rs + montanha-tcp | 2026-08-28 |
| P0.2 | p0 | leave must commit | done | queued_leave_after_joint_must_be_finished | 2026-08-28 |
| P1.1 | p1 | LeaveJoint worker | done | Work::LeaveJoint | 2026-08-28 |
| P1.2 | p1 | 3-process leave in log | done | l28_real_tcp_remove_member_left_on_disk | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | RFC-0123 | 2026-08-28 |
| P2.2 | p2 | not ∀ traces | done | queued_leave_finish_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `queued_leave_finish_ok(true, false)` false; AS-IS true; `(true, true)` true; `(false, _)` true.
  - `queued_leave_after_joint_must_be_finished`: Queued shrink; finish the joint propose; leave is in the log with `index > commit`; `finish_uncommitted_leave` + pump until `uncommitted_leave_index` is None. 0121 joint-only finish is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.2 `queued_leave_finish_campaign_is_not_forall_traces`: `R-joint` and `R-swarm-real` stay continuous; catalog pair `queued_leave_finish` / `queued_leave_finish_ok`; campaign not a theorem.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0121 P1.2 stays 3-process disk; `residuals.json` `R-joint` owner 0122.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
