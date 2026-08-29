# RFC: 0120 — TCP `RemoveMemberJoint` must call production `remove_member_joint`

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0119](0119-tcp-replica-joint-target.md), [0117](0117-tcp-leave-joint.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G4 real TCP + G7 reconfig**. RFC-0119 lets a TCP replica name a peer that is not in local `nodes`. There is still no wire tag to *ask* `remove_member_joint` over MTCP. AS-IS tag 19 is `tcp bad tag`. This slice: tag 19 `RemoveMemberJoint { node_id }`, `client_remove_member_joint`, TCP worker → production `remove_member_joint`. Planting until leave on 3-process `cluster_real` is **not** this tooth (0118 P1.1 / 0066 P2.2). AddMember wire is P2.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Tags 1–18 exist; 18 is LeaveJoint.
- `remove_member_joint` is production; TCP never invoked it.
- 0119 P1.2 named this wire.

## Problems This Solves

- **Problem:** no TCP way to start a joint remove.
- **Problem:** 0118 P1.1 plant is blocked on missing wire.
- **Problem:** AS-IS tag 19 is a decode error.

## Proposed Solution

- `WireMsg::RemoveMemberJoint { node_id }` tag 19. `client_remove_member_joint`. `montanha-tcp` worker → `remove_member_joint`. Named encode/decode test. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (wire + client + worker)
- [x] **P0.1** Tag 19 RemoveMemberJoint + `client_remove_member_joint` + TCP worker — status: `done`
- [x] **P0.2** Regression — status: `done` (`wire_remove_member_joint_round_trip`)

### P1 — next wave
- [x] **P1.1** CLI `montanha-tcp remove-member-joint` — status: `done` (`cmd_remove_member_joint`)
- [x] **P1.2** `cluster_real` plant remove then leave (0066 P2.2) — status: `done` (RFC-0121 P0 invoke; RFC-0121 P1.2 on-disk `l28_real_tcp_remove_member_left_on_disk`)

### P2 — later
- [x] **P2.1** Tag 20 AddMemberJoint — status: `done` (`wire_add_member_joint_round_trip`)
- [x] **P2.2** Campaign is not ∀ traces — status: `done` (`joint_target_campaign_is_not_forall_traces`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RemoveMemberJoint wire + worker | done | tcp.rs + montanha-tcp.rs | 2026-08-28 |
| P0.2 | p0 | tag 19 round-trip | done | wire_remove_member_joint_round_trip | 2026-08-28 |
| P1.1 | p1 | CLI remove-member-joint | done | cmd_remove_member_joint | 2026-08-28 |
| P1.2 | p1 | cluster_real plant+leave | done | RFC-0121 P1.2 l28_real_tcp_remove_member_left_on_disk | 2026-08-28 |
| P2.1 | p2 | AddMemberJoint wire | done | wire_add_member_joint_round_trip | 2026-08-28 |
| P2.2 | p2 | not ∀ traces | done | joint_target_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `wire_remove_member_joint_round_trip`: encode/decode identity; body is `[19]` + u64 LE id; tag 0 still `tcp bad tag`. Runs on Darwin. Does not submit io_uring SQEs.
  - Production `montanha-tcp` maps `WireMsg::RemoveMemberJoint` to `StoreCluster::remove_member_joint`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0119 P1.2 wire; `residuals.json` `R-joint` owner 0120.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process plant until leave (P1.2).
- Rocks coluna A/B. crates.io.
