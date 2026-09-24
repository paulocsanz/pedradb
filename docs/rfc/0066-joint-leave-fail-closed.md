# RFC: 0066 — Leave-joint fail-closed (C-new only after C-old,new commits)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0064](0064-joint-election-fail-closed.md), [0063](0063-fdb-reliability-close-the-system-gap.md), [0061](0061-residuals-sel4-ironfleet.md), [0051](0051-beyond-fdb-sim-holes.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig mid-run** (coverage-map “Reconfig out-of-band sem joint”; G7 operator/coordinator change is outside `fdbserver -r simulation`). This slice closes one named hole: after `MembershipJoint` **commits**, election/commit still used C-old only until apply, because `pending_joint` ignored `index <= commit`. Raft §6 requires C-old∧C-new until a **leave** entry (C-new only) is committed.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. It names one Sim2-class hole and a kernel+handler close.

## Background

- RFC-0063/0064: log-carried `MembershipJoint`, commit and election old∧new **while the joint is uncommitted** (`rec.index > commit`).
- `pending_joint_on` dropped the joint the moment `commit >= joint_index`. Between commit and apply, `ids` is still C-old → old-only majority elects. After apply, `ids` jumps to C-new with **no** C-new-only log entry (Ongaro phase 2).
- A second reconfig was allowed as soon as apply ran, overlapping the still-joint log prefix.

## Problems This Solves

- **Problem:** committed joint + not-yet-applied = election on C-old (the 0064 hole, one index later).
- **Problem:** no leave-joint (`old == new`) means the log never records “we left C-old,new”.
- **Problem:** `R-joint` stayed `open` while 0063/0064 marked P0–P2 done — catalog lied.

## Proposed Solution

- Pure `joint_still_active(old, new)` (`old != new`). AS-IS always `false` (dente: treat committed joint as single config).
- Production: active joint = uncommitted joint **or** latest committed `MembershipJoint` with `old != new` not superseded by a committed leave.
- After a joint commits, the leader appends leave `MembershipJoint { old: new, new }` (C-new only). Election/commit keep old∧new until that leave commits.

## Delivery slices (mandatory)

### P0 — must ship first (leave-joint on the live store path)
- [x] **P0.1** `membership_kernel::joint_still_active` + AS-IS + store clone — status: `done`
- [x] **P0.2** Store `pending_joint_on` sees committed-but-not-left joints; leader appends leave after joint commit — status: `done`
- [x] **P0.3** Regression: committed joint add, C-old majority **does not** elect; AS-IS would — status: `done` (`election_after_committed_joint_still_requires_new_majority`)

### P1 — next wave
- [x] **P1.1** Stateright model includes leave-joint (dente AS-IS elects after commit) — status: `done` (RFC-0094)
- [x] **P1.2** World `Action` that plants committed-without-leave and checks the tooth — status: `done` (RFC-0068 P0 `PlantCommittedJoint`)

### P2 — later
- [x] **P2.1** Verus twin of `joint_still_active` — status: `done` (RFC-0095)
- [x] **P2.2** L28 REAL: leave-joint on `cluster_real` — status: `done` (RFC-0121 P1.2 `l28_real_tcp_remove_member_left_on_disk`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | joint_still_active kernel + AS-IS | done | membership_kernel.rs (raft+store) | 2026-08-27 |
| P0.2 | p0 | pending joint until leave; auto-leave | done | pending_joint_on + leave_joint | 2026-08-27 |
| P0.3 | p0 | committed joint old-only tooth | done | election_after_committed_joint_still_requires_new_majority | 2026-08-27 |
| P1.1 | p1 | Stateright leave-joint | done | RFC-0094 joint_leave_model | 2026-08-28 |
| P1.2 | p1 | World committed-without-leave | done | RFC-0068 PlantCommittedJoint | 2026-08-27 |
| P2.1 | p2 | Verus twin | done | RFC-0095 membership_joint.rs | 2026-08-28 |
| P2.2 | p2 | L28 REAL leave | done | RFC-0121 P1.2 l28_real_tcp_remove_member_left_on_disk | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_still_active` is true iff `old != new`; AS-IS is always false.
  - `election_after_committed_joint_still_requires_new_majority`: C-old=3, committed joint add 4th, applied behind commit, 2 C-old votes → `election_has_joint_quorum` false; `joint_still_active_as_is` would ignore the joint.
  - Existing `election_during_joint_add_refuses_old_only_majority` and `log_carried_joint_remove_crosses_out_of_band_floor` stay green (leave is transparent after Direct apply).
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; `residuals.json` `R-joint` close-text + class; 0061 pointer.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Extracting `db.rs` (L46). Flow (L29). Proving Linux/`fsync`/CPU/`rustc`.
- Closing `never_floor` (`R-cpu` … `R-extract`).
- RFC-0065 P2 raftdb/cache. Rocks coluna A/B. crates.io.
- Two removes in one entry (0064 out of scope, still).
