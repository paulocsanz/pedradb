# RFC: 0096 — Live `add_member_joint` must leave a C-new-only log entry

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0095](0095-verus-joint-still-active.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. `leave_joint` appends `MembershipJoint { old: new, new }` after a joint **commits**. Stateright (0094) and the Verus twin (0095) do not watch the live store log. AS-IS `joint_leave_ok` treats leave as optional (skip). This slice names the gate on the **live `add_member_joint` path**: a C-new-only entry must be in the log. `cluster_real` 3-process leave is **not** this tooth (0066 P2.2).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `add_member_joint` / `remove_member_joint` call `leave_joint` after append.
- RFC-0066 P0 shipped the helper; no named test required the leave bytes on the live log.
- Clone `membership_raft_store` lists kernel fns; `joint_leave_ok` joins that list.

## Problems This Solves

- **Problem:** leave could be skipped and the add still look “applied.”
- **Problem:** AS-IS admits any log as if leave happened.
- **Problem:** 0094/0095 never opened a `StoreCluster`.

## Proposed Solution

- Pure `joint_leave_ok(leave_in_log)` = the bool. AS-IS always true. Production `leave_joint` after a planted committed joint must write a leave; the named test reads the leader log. Compacted `add_member_joint` logs are not this tooth. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live add)
- [x] **P0.1** `joint_leave_ok` + AS-IS + store clone — status: `done`
- [x] **P0.2** Regression: live `leave_joint` after plant, leave in log — status: `done` (`leave_joint_on_live_store_is_in_log`)

### P1 — next wave
- [ ] **P1.1** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`
- [ ] **P1.2** Verus twin of `joint_leave_ok` — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [x] **P2.2** Queued RPC leave (not only Direct lab) — status: `done` (RFC-0097)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | joint_leave_ok + clone | done | membership_kernel.rs raft+store | 2026-08-28 |
| P0.2 | p0 | live leave in log | done | leave_joint_on_live_store_is_in_log | 2026-08-28 |
| P1.1 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P1.2 | p1 | Verus twin | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | Queued leave | done | RFC-0097 | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_leave_ok(false)` false; AS-IS true.
  - `leave_joint_on_live_store_is_in_log`: production `plant_committed_joint_without_leave` then `leave_joint` (Direct, Darwin). Leader log contains `MembershipJoint` with `!joint_still_active`. `joint_leave_ok` is true. AS-IS would pass without that entry. Compacted `add_member_joint` logs are **not** this tooth. Does not submit io_uring SQEs.
  - Existing `election_after_committed_joint_still_requires_new_majority` (no leave planted) is **not** this tooth.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; `residuals.json` `R-joint` owner 0096; RFC-0095 P1.1 stays `cluster_real`.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave (P1.1).
- Rocks coluna A/B. crates.io.
