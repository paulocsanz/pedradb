# RFC: 0119 — TCP replica `remove_member_joint` must not require local `nodes`

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0118](0118-cluster-real-tcp-leave.md), [0066](0066-joint-leave-fail-closed.md), [0063](0063-fdb-reliability-close-the-system-gap.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G4 real TCP + G7 reconfig**. RFC-0118 invokes `leave_joint` on 3-process TCP (no-op when no joint). A committed C-old,new then leave is still 0066 P2.2 / 0118 P1.1. That plant is blocked: `remove_member_joint` returns `unknown node` unless the target is in the local `nodes` map. `open_single_node` / `montanha-tcp` only inserts `self`. AS-IS `joint_target_counts` is `in_nodes`. This slice: membership set (`ids`) is enough to start a joint remove. Add of a never-local node is **not** this tooth (P1).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- TCP node is `open_single_node`: `ids` = mesh, `nodes` = `{self}`.
- `remove_member_joint` checked `nodes.contains_key` first. Peer 3 on process 1 is always unknown.
- In-process `StoreCluster::open` has every id in `nodes`, so 0099–0116 never saw the hole.

## Problems This Solves

- **Problem:** REAL TCP cannot start a joint remove.
- **Problem:** 0118 P1.1 committed-joint-then-leave is blocked on plant.
- **Problem:** AS-IS requires the target in local `nodes`.

## Proposed Solution

- Pure `joint_target_counts(in_ids, in_nodes)` = `in_ids`. AS-IS = `in_nodes`. Production `remove_member_joint` after the idempotent `!ids` Ok. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (TCP replica can name a peer)
- [x] **P0.1** `joint_target_counts` + `remove_member_joint` — status: `done`
- [x] **P0.2** Regression — status: `done` (`remove_member_joint_tcp_replica_does_not_require_local_nodes`)

### P1 — next wave
- [x] **P1.1** `add_member_joint` same hole (never-local joining node) — status: `done`
- [x] **P1.2** TCP wire Add/Remove so `cluster_real` can plant — status: `done` (RFC-0120 P0 tag 19 RemoveMemberJoint; tag 20 AddMemberJoint `wire_add_member_joint_round_trip`)

### P2 — later
- [x] **P2.1** Verus twin of `joint_target_counts` — status: `done` (catalog `joint_target` / `joint_add_target`)
- [x] **P2.2** Campaign is not ∀ traces — status: `done` (`joint_target_campaign_is_not_forall_traces`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | joint_target_counts + remove | done | membership_kernel + remove_member_joint | 2026-08-28 |
| P0.2 | p0 | TCP replica remove named | done | remove_member_joint_tcp_replica_does_not_require_local_nodes | 2026-08-28 |
| P1.1 | p1 | add never-local | done | add_member_joint_tcp_replica_does_not_require_local_nodes | 2026-08-28 |
| P1.2 | p1 | TCP add/remove wire | done | tag 19 RFC-0120; tag 20 wire_add_member_joint_round_trip | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | catalog joint_target + joint_add_target | 2026-08-28 |
| P2.2 | p2 | not ∀ traces | done | joint_target_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_target_counts(true, false)` true; AS-IS false; `(false, true)` false; AS-IS true.
  - `remove_member_joint_tcp_replica_does_not_require_local_nodes`: `open_single_node(self=1, ids=[1,2,3])`; node 3 not in `nodes`; `remove_member_joint(3)` is **not** `unknown node` (NotLeader is ok — this tooth is the map check). Runs on Darwin. Does not submit io_uring SQEs.
  - `add_member_joint_tcp_replica_does_not_require_local_nodes`: `open_single_node(self=1, ids=[1,2,3])`; node 4 not in `nodes`/`ids`; `add_member_joint(4)` is **not** `unknown node`.
  - In-process 4-node shrink/add tests stay green (`in_ids ∧ in_nodes`).
  - P1.2 `wire_add_member_joint_round_trip` / `wire_remove_member_joint_round_trip`: tags 20/19 + u64 LE; unknown tag fail-closed. CLI `montanha-tcp add-member-joint` / `remove-member-joint`. Production worker calls `add_member_joint` / `remove_member_joint`. 3-process plant until leave is 0066 P2.2 / 0120 P1.2.
  - P2.1 catalog pairs `joint_target` / `joint_add_target`; freeze twins of `joint_target_counts` / `joint_add_target_counts`.
  - P2.2 `joint_target_campaign_is_not_forall_traces`: `R-joint` campaign not a theorem.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0118 P1.1 stays committed-joint-then-leave; `residuals.json` `R-joint` owner 0119.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Planting a joint over 3-process TCP until leave (0066 P2.2 / 0120 P1.2).
- Rocks coluna A/B. crates.io.
