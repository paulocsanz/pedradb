# RFC: 0121 — TCP joint remove must finish the queued propose (same as put)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0120](0120-tcp-remove-member-joint.md), [0118](0118-cluster-real-tcp-leave.md), [0098](0098-queued-add-member-leave.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` + `R-swarm-real` (not `never_floor`). Axis vs FDB Sim2: **G4 real TCP + G7 reconfig**. RFC-0120 ships tag 19 and the worker calling `remove_member_joint` once. Production TCP is Queued: `put` drives `finish_queued_propose` on `NotCommitted`; membership does not. The client sees `NotCommitted` and a retry hits “joint already in flight”. `finish_queued_propose` is what appends leave (0098). AS-IS `l28_tcp_plant_ok` is always true. This slice: worker uses the same finish path as put; `cluster_real --remove-member` after the L28 triple calls `client_remove_member_joint` then leave; exit via the kernel. P1.2 is the 3-process on-disk C-new-only scan after plant (0066 P2.2).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `put_many_until_committed` / `finish_not_committed` already exist on `montanha-tcp`.
- `remove_member_joint` on Queued returns `NotCommitted` after the joint is on the leader log; `leave_joint_after_commit` is not reached (`?`).
- `finish_queued_propose` calls `leave_joint_after_commit` after the joint commits.

## Problems This Solves

- **Problem:** TCP joint remove does not finish the queued propose.
- **Problem:** 0120 P1.2 plant cannot commit.
- **Problem:** AS-IS skips the plant flag.

## Proposed Solution

- Worker: `NotCommitted` from add/remove → `finish_not_committed` (same as put). `l28_tcp_plant_ok(remove, leave)` = and. AS-IS true. `cluster_real --remove-member` after restart. Named kernel test. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (finish queued joint)
- [x] **P0.1** Worker finish + `cluster_real --remove-member` + kernel — status: `done`
- [x] **P0.2** Regression — status: `done` (`l28_tcp_plant_ok_requires_remove_and_leave`)

### P1 — next wave
- [x] **P1.1** AddMemberJoint uses the same finish path — status: `done`
- [x] **P1.2** 3-process log has C-new-only after plant (0066 P2.2) — status: `done` (`l28_real_tcp_remove_member_left_on_disk`)

### P2 — later
- [x] **P2.1** Campaign is not ∀ traces — status: `done` (`l28_tcp_plant_campaign_is_not_forall_traces`)
- [x] **P2.2** R-verus still never — status: `done` (`l28_tcp_plant_verus_still_never`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | finish queued + cluster_real flag | done | montanha-tcp + cluster_real + l28.rs | 2026-08-28 |
| P0.2 | p0 | plant kernel | done | l28_tcp_plant_ok_requires_remove_and_leave | 2026-08-28 |
| P1.1 | p1 | add finish path | done | Work::AddMemberJoint | 2026-08-28 |
| P1.2 | p1 | 3-process leave in log | done | l28_real_tcp_remove_member_left_on_disk | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | done | l28_tcp_plant_campaign_is_not_forall_traces | 2026-08-28 |
| P2.2 | p2 | R-verus never | done | l28_tcp_plant_verus_still_never | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `l28_tcp_plant_ok(false, true)` false; `(true, false)` false; AS-IS true.
  - Production `montanha-tcp` drives `finish_not_committed` on `NotCommitted` from `remove_member_joint` / `add_member_joint`.
  - `cluster_real --remove-member` after the durability triple calls `client_remove_member_joint` then `client_leave_joint`; default fingerprint omits `remove=`/`leave=` from this flag. Exit via `l28_tcp_plant_ok`.
  - P1.2 `l28_real_tcp_remove_member_left_on_disk`: seed `0x0121_1E28` twice with `--remove-member`; fingerprints match; `remove=1` `leave=1` `left=1`; `l28_durability_ok` ∧ `l28_tcp_plant_ok` ∧ `l28_tcp_left_ok`. After kids die, `tcp_node_disk_left_joint` finds C-new-only (leave in recovered log, or membership already C-new with no still-active joint after compact). AS-IS would skip the scan. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 `l28_tcp_plant_campaign_is_not_forall_traces`: `R-joint` and `R-swarm-real` stay continuous; close names `l28_tcp_left_ok` / `l28_real_tcp_remove_member_left_on_disk`; campaign not a theorem.
  - P2.2 `l28_tcp_plant_verus_still_never`: `R-verus` stays in `never_floor`; catalog pair `l28_tcp_left` / twin `l28_tcp_left_ok` is freeze, not a verified verifier. Does not run `verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0120 P1.2 invoke; `residuals.json` `R-joint` owner 0121.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0 (slow unix; 0066 P2.2).
- Rocks coluna A/B. crates.io.
