# RFC: 0144 — No-leader discard persist-leader must be a local node

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0143](0143-discard-uncommitted-local-non-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. `finish_queued_propose` with no range leader still passes `ids.first()` into `discard_uncommitted_from` as the persist-leader. After leave, TCP `ids.first()` is a **remote** voter: node 4 truncate persist is best-effort (`let _ = persist_log_db`) and the leader-only `next_index` repair is skipped. AS-IS `discard_leader_local` is true regardless of locality. This slice: persist-leader is the first **local** id (`ids` then `nodes`). 0143 loop membership is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- No-leader abort: `discard_uncommitted_from(range, ids.first(), index)`.
- Persist is fail-closed only when `nid == leader`.

## Problems This Solves

- **Problem:** TCP removed replica persist of the truncated log is swallowed.
- **Problem:** `next_index` repair never runs on the local peer.
- **Problem:** AS-IS persist-leader is `ids.first()` even when remote.

## Proposed Solution

- Pure `discard_leader_local(is_local)` = `is_local`. AS-IS true. No-leader path finds the first local id. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (TCP removed persist-leader)
- [x] **P0.1** `discard_leader_local` + no-leader pick — status: `done`
- [x] **P0.2** Regression — status: `done` (`finish_queued_no_leader_persist_leader_is_local`)

### P1 — next wave
- [x] **P1.1** TCP remaining member still uses local `ids.first()` — status: `done` (`finish_queued_member_persist_leader_stays_first`)
- [x] **P1.2** TCP 3-process — status: `done` (`l28_real_tcp_removed_pld`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | persist-leader is local | done | finish_queued_propose | 2026-08-28 |
| P0.2 | p0 | TCP removed next_index repaired | done | finish_queued_no_leader_persist_leader_is_local | 2026-08-28 |
| P1.1 | p1 | TCP member ids.first local | done | finish_queued_member_persist_leader_stays_first | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_pld | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + discard_leader | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | discard_leader_local_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `discard_leader_local(false)` false; AS-IS true. Same tokens raft=store.
  - `finish_queued_no_leader_persist_leader_is_local`: Queued 4→3 leave; `open_single_node(4)`; plant suffix; `finish_queued_propose(..., abort)`; `next_index` on node 4 for remaining members is `from`. AS-IS remote `ids.first()` skips the repair. 0143 direct `discard_uncommitted_from(leader=4)` is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `finish_queued_member_persist_leader_stays_first`: TCP ctor of node 1 after leave; no-leader abort still picks 1 (local `ids.first()`). removed ctor is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_pld`: seed `0x0144_1E28` twice with `--remove-member`; fingerprints match; `pld=1`; production TCP ctor on the **removed** replica plants an uncommitted suffix then `finish_queued_propose(..., abort)`; `next_index` for remaining member 1 is `from`; `!is_member` and no local range leader. 0143 direct discard is **not** this tooth. Exit via `l28_tcp_pld_ok`. AS-IS remote `ids.first()` skips the repair. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `discard_leader` `entry: discard_leader_local`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `discard_leader_local_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `discard_leader_local_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0143 P1.2 TCP live discard is **done**; `residuals.json` `R-joint` owner 0144.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
