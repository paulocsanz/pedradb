# RFC: 0145 — A removed replica must not remain Role::Leader

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0144](0144-discard-persist-leader-local.md), [0128](0128-is-participating-requires-ids.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. Out-of-band `remove_member` steps a removed node down from Leader. Log-carried joint leave goes through `install_applied_membership`, which only flips `participating`. A leader that is then dropped from `ids` keeps `Role::Leader` in RAM (`node_thinks_leader` true). AS-IS `removed_steps_down` is false. This slice: apply of C-new steps the removed replica down. 0144 persist-leader is **not** this tooth. 0128 `is_participating` is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `range_leader` already requires participating, so clients fail closed.
- `node_thinks_leader` is raw `Role::Leader`.

## Problems This Solves

- **Problem:** joint leave leaves a stale Leader on the removed replica.
- **Problem:** oob `remove_member` steps down; log-carried leave does not.
- **Problem:** AS-IS keep Role::Leader after C-new apply.

## Proposed Solution

- Pure `removed_steps_down(in_ids)` = `!in_ids`. AS-IS false. `install_applied_membership` steps down local non-members. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (live joint leave)
- [x] **P0.1** `removed_steps_down` + install step-down — status: `done`
- [x] **P0.2** Regression — status: `done` (`leave_steps_down_removed_leader`)

### P1 — next wave
- [x] **P1.1** Remaining members still elect a member leader — status: `done` (`leave_remaining_elect_member_leader`)
- [x] **P1.2** TCP 3-process — status: `done` (`l28_real_tcp_removed_std`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | step down on C-new apply | done | install_applied_membership | 2026-08-28 |
| P0.2 | p0 | removed leader is follower | done | leave_steps_down_removed_leader | 2026-08-28 |
| P1.1 | p1 | remaining elect a member | done | leave_remaining_elect_member_leader | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_std | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + removed_step_down | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | removed_steps_down_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `removed_steps_down(false)` true; AS-IS false. Same tokens raft=store.
  - `leave_steps_down_removed_leader`: Queued 4→3 leave; plant stale `Role::Leader` on node 4; `install_applied_membership(C-new)`; `!node_thinks_leader(4,1)`. 0128 participating is **not** this tooth. oob `remove_member` step-down is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `leave_remaining_elect_member_leader`: after that leave, `elect_all`; `range_leader` is a remaining member; 4 still not leader.
  - P1.2 `l28_real_tcp_removed_std`: seed `0x0145_1E28` twice with `--remove-member`; fingerprints match; `std=1`; production TCP ctor on the **removed** replica plants `Role::Leader` then re-installs C-new; `!node_thinks_leader` and `!is_member`. 0144 persist-leader is **not** this tooth. 0128 participating is **not** this tooth. Exit via `l28_tcp_std_ok`. AS-IS keeps `Role::Leader`. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `removed_step_down` `entry: removed_steps_down`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `removed_steps_down_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `removed_steps_down_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0144 P1.2 TCP persist-leader is **done**; `residuals.json` `R-joint` owner 0145.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
