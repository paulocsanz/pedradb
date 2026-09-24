# RFC: 0146 — Leader routing hint must be a current member

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0145](0145-removed-steps-down.md), [0128](0128-is-participating-requires-ids.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. After joint leave, `range_leader` already requires participating. `leader_hint` then falls back to any local peer’s `leader_id`, which may still name the **removed** replica. `put` surfaces that id in `NotLeader` and status JSON routes clients to a non-member. AS-IS `hint_if_member` is true regardless of `ids`. This slice: a hint counts only if it is in `ids`. 0145 Role::Leader step-down is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `leader_hint` is the client retry / status routing id when there is no live `range_leader`.
- Remaining peers can keep `leader_id = Some(removed)`.

## Problems This Solves

- **Problem:** clients are told to retry on a replica that already left.
- **Problem:** status JSON can print the removed node as leader.
- **Problem:** AS-IS any `leader_id` is returned.

## Proposed Solution

- Pure `hint_if_member(in_ids)` = `in_ids`. AS-IS true. `leader_hint` filters; `install_applied_membership` clears stale hints. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (`leader_hint`)
- [x] **P0.1** `hint_if_member` + hint filter — status: `done`
- [x] **P0.2** Regression — status: `done` (`leader_hint_omits_removed_after_leave`)

### P1 — next wave
- [x] **P1.1** Apply C-new clears stale hints on remaining peers — status: `done` (`install_clears_stale_leader_hint`)
- [x] **P1.2** TCP 3-process — status: `done` (`l28_real_tcp_hint`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | hint requires member | done | leader_hint | 2026-08-28 |
| P0.2 | p0 | hint is not the removed node | done | leader_hint_omits_removed_after_leave | 2026-08-28 |
| P1.1 | p1 | apply clears stale hint | done | install_clears_stale_leader_hint | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_hint | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + hint_member | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | hint_if_member_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `hint_if_member(false)` false; AS-IS true. Same tokens raft=store.
  - `leader_hint_omits_removed_after_leave`: Queued 4→3 leave; no live `range_leader`; plant `leader_id=4` on remaining peers; `leader_hint(1) != Some(4)`. 0145 step-down is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `install_clears_stale_leader_hint`: after leave, plant `leader_id=4` on node 1; `install_applied_membership(C-new)`; node 1 hint is not 4. `leader_hint` filter is **not** this tooth.
  - P1.2 `l28_real_tcp_hint`: seed `0x0146_1E28` twice with `--remove-member`; fingerprints match; `hnt=1`; production TCP ctor of a **remaining** voter (n1) plants `leader_id=3`; `leader_hint(1) != Some(3)`; n1 is a member and 3 is not. 0145 step-down is **not** this tooth. Exit via `l28_tcp_hnt_ok`. AS-IS returns any `leader_id`. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `hint_member` `entry: hint_if_member`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `hint_if_member_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `hint_if_member_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0145 P1.2 TCP Leader step-down is **done**; `residuals.json` `R-joint` owner 0146.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
