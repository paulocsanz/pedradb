# RFC: 0123 — Production must call `queued_leave_finish_ok`; Verus twin

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0122](0122-tcp-leave-finish-queued.md), [0110](0110-verus-joint-leave-ok.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig** + **G1 process death**. RFC-0122 named `queued_leave_finish_ok` and `finish_uncommitted_leave`, but production never **calls** the kernel (tests only). Freeze would still pass if the gate vanished. The Verus twin has `joint_leave_ok` (0110) but not this commit gate. This slice: production `finish_uncommitted_leave` / TCP drive fail-close via the kernel; catalog pair + twin. Crash-reopen of a committed leave is P1. A Verus twin is not a verified verifier (R-verus). 0122 RAM finish is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `queued_leave_finish_ok(in_log, committed) = !in_log || committed`. AS-IS = `in_log`.
- TCP drive already errors if `uncommitted_leave_index` stays Some, without the kernel.
- Twin `membership_joint.rs` lacks this fn.

## Problems This Solves

- **Problem:** kernel lived only in tests.
- **Problem:** freeze twins would pass if the gate vanished from the twin.
- **Problem:** AS-IS “in the log” had no twin lemma.

## Proposed Solution

- `finish_uncommitted_leave` returns the kernel. TCP drive fail-closes with the kernel. Twin exec + catalog pair `queued_leave_finish`. Named glue already `queued_leave_finish_ok_requires_commit`. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (production calls the kernel + twin)
- [x] **P0.1** Production + TCP drive call `queued_leave_finish_ok` — status: `done`
- [x] **P0.2** Verus twin + catalog pair — status: `done`

### P1 — next wave
- [x] **P1.1** Committed leave survives `crash_reopen_engine_on` — status: `done` (`queued_leave_survives_crash_reopen`)
- [x] **P1.2** 3-process C-new-only on disk (0066 P2.2) — status: `done` (`l28_real_tcp_remove_member_left_on_disk`)

### P2 — later
- [x] **P2.1** Campaign is not ∀ traces — status: `done` (`queued_leave_finish_twin_campaign_is_not_forall_traces`)
- [x] **P2.2** R-verus still never — status: `done` (`queued_leave_finish_verus_still_never`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | production calls kernel | done | finish_uncommitted_leave + TCP drive | 2026-08-28 |
| P0.2 | p0 | Verus twin + catalog | done | membership_joint.rs + queued_leave_finish | 2026-08-28 |
| P1.1 | p1 | crash-reopen keeps leave | done | queued_leave_survives_crash_reopen | 2026-08-28 |
| P1.2 | p1 | 3-process leave in log | done | l28_real_tcp_remove_member_left_on_disk | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | done | queued_leave_finish_twin_campaign_is_not_forall_traces | 2026-08-28 |
| P2.2 | p2 | R-verus never | done | queued_leave_finish_verus_still_never | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `queued_leave_finish_ok_requires_commit` stays green.
  - Catalog pair `queued_leave_finish` `entry: queued_leave_finish_ok`; freeze twins fail if the exec fn is dropped from the twin.
  - `queued_leave_survives_crash_reopen`: after 0122 finish, persist + `crash_reopen_engine_on` leader; C-new-only still in the reopened log with `index <= commit` (or snapshot covers it). 0122 RAM-only is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - `./scripts/verus_membership_joint.sh` when Verus is installed (not run here; freeze of the twin file).
  - P2.1 `queued_leave_finish_twin_campaign_is_not_forall_traces`: `R-joint` stays continuous; catalog pair `queued_leave_finish` + twin `membership_joint.rs` `queued_leave_finish_ok`; campaign not a theorem. Does not run `verus`.
  - P2.2 `queued_leave_finish_verus_still_never`: `R-verus` stays in `never_floor`; catalog/twin freeze of `queued_leave_finish_ok` is not a verified verifier. Does not run `verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0122 P2.1 done; `residuals.json` `R-joint` owner 0123.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
