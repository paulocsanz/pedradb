# RFC: 0117 — TCP `LeaveJoint` must call production `leave_joint`

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0116](0116-verus-election-grant-from.md), [0066](0066-joint-leave-fail-closed.md), [0017](0017-tcp-wire.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`) + **G4 real TCP**. Axis vs FDB Sim2: membership leave is in-process only; `cluster_real` / `montanha-tcp` had no client command, so 3-process L28 leave (0066 P2.2) could not even be asked. AS-IS unknown tag 18 is `tcp bad tag`. This slice ships wire tag 18 `LeaveJoint`, `client_leave_joint`, and the TCP worker calling `StoreCluster::leave_joint`. 3-process `cluster_real --leave-joint` is **not** this tooth (P1).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Wire tags 1–17 exist; decode of unknown tags is `tcp bad tag`.
- `leave_joint` is production on `StoreCluster`; TCP never invoked it.

## Problems This Solves

- **Problem:** no TCP way to leave a joint.
- **Problem:** cluster_real leave was blocked on missing wire.
- **Problem:** AS-IS tag 18 is a decode error.

## Proposed Solution

- `WireMsg::LeaveJoint` tag 18. `client_leave_joint`. `montanha-tcp` worker → `leave_joint`. Named encode/decode test. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (wire + client + worker)
- [x] **P0.1** Tag 18 LeaveJoint + `client_leave_joint` + TCP worker — status: `done`
- [x] **P0.2** Regression — status: `done` (`wire_leave_joint_round_trip`)

### P1 — next wave
- [ ] **P1.1** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`
- [x] **P1.2** Campaign is not ∀ traces — status: `done` (RFC-0118 P2.1)

### P2 — later
- [x] **P2.1** R-verus still never — status: `done` (RFC-0118 P2.2)
- [x] **P2.2** CLI `montanha-tcp leave-joint` — status: `done` (RFC-0118)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | LeaveJoint wire + worker | done | tcp.rs + montanha-tcp.rs | 2026-08-28 |
| P0.2 | p0 | tag 18 round-trip | done | wire_leave_joint_round_trip | 2026-08-28 |
| P1.1 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P1.2 | p1 | not ∀ traces | done | RFC-0118 P2.1 | 2026-08-28 |
| P2.1 | p2 | R-verus never | done | RFC-0118 P2.2 | 2026-08-28 |
| P2.2 | p2 | CLI leave-joint | done | RFC-0118 cmd_leave_joint | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `wire_leave_joint_round_trip`: `LeaveJoint` encode/decode identity; body is `[18]`; tag 0 still `tcp bad tag`. Runs on Darwin. Does not submit io_uring SQEs.
  - Production `montanha-tcp` maps `WireMsg::LeaveJoint` to `StoreCluster::leave_joint`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0118 P0 invokes leave on `cluster_real`; `residuals.json` `R-joint` owner 0118.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave (P1.1).
- Rocks coluna A/B. crates.io.
