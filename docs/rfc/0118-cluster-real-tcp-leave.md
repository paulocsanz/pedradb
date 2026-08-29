# RFC: 0118 — REAL TCP `cluster_real` must invoke `leave_joint`

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0117](0117-tcp-leave-joint.md), [0072](0072-l28-real-tcp-durability-kernel.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` + `R-swarm-real` (not `never_floor`). Axis vs FDB Sim2: **G4 real TCP**. RFC-0117 shipped tag 18 and `client_leave_joint`. `cluster_real` never called it, so 3-process L28 leave (0066 P2.2) was still unasked. AS-IS `l28_tcp_leave_ok` is always true. This slice: CLI `montanha-tcp leave-joint`, `cluster_real --leave-joint` after elect calls `client_leave_joint` (no-op Ok when no joint is in flight), exit via the kernel. A committed C-old,new then leave on REAL TCP is **not** this tooth (still 0066 P2.2).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `leave_joint` with no pending joint returns Ok without append.
- Production `StoreCluster::open` is Queued; no-op leave does not hit NotCommitted.
- 0117 wire is unused by `cluster_real`.

## Problems This Solves

- **Problem:** REAL TCP never called leave.
- **Problem:** no CLI to drive tag 18.
- **Problem:** AS-IS would skip the TCP leave flag.

## Proposed Solution

- `l28_tcp_leave_ok(tcp_ok)` = the bool. AS-IS true. CLI + `cluster_real --leave-joint`. Named kernel test. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (TCP leave invoked)
- [x] **P0.1** CLI + `cluster_real --leave-joint` + kernel — status: `done`
- [x] **P0.2** Regression — status: `done` (`l28_tcp_leave_ok_requires_tcp_ok`)

### P1 — next wave
- [x] **P1.1** L28 REAL leave of a **committed joint** (0066 P2.2) — status: `done` (RFC-0121 P1.2)
- [x] **P1.2** `l28_real_tcp` `--leave-joint` replay — status: `done` (`l28_real_tcp_leave_joint_replay`)

### P2 — later
- [x] **P2.1** Campaign is not ∀ traces — status: `done` (`l28_tcp_leave_campaign_is_not_forall_traces`)
- [x] **P2.2** R-verus still never — status: `done` (`l28_tcp_leave_verus_still_never`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | CLI + cluster_real TCP leave | done | montanha-tcp + cluster_real + l28.rs | 2026-08-28 |
| P0.2 | p0 | tcp leave kernel | done | l28_tcp_leave_ok_requires_tcp_ok | 2026-08-28 |
| P1.1 | p1 | REAL joint then leave | done | RFC-0121 P1.2 l28_real_tcp_remove_member_left_on_disk | 2026-08-28 |
| P1.2 | p1 | l28_real_tcp leave replay | done | l28_real_tcp_leave_joint_replay | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | done | l28_tcp_leave_campaign_is_not_forall_traces | 2026-08-28 |
| P2.2 | p2 | R-verus never | done | l28_tcp_leave_verus_still_never | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `l28_tcp_leave_ok(false)` false; AS-IS true.
  - `cluster_real --leave-joint` after elect calls `client_leave_joint` and exits via `l28_tcp_leave_ok`.
  - `montanha-tcp leave-joint --addr` calls `client_leave_joint`.
  - A committed joint then leave on 3-process TCP is **not** this tooth. Runs on Darwin for the kernel test. Does not submit io_uring SQEs.
  - P1.2 `l28_real_tcp_leave_joint_replay`: seed `0x0118_1E28` twice with `--leave-joint`; fingerprints match; `leave=1`; `l28_durability_ok` ∧ `l28_tcp_leave_ok`; AS-IS would skip leave. Committed-joint-then-leave is RFC-0121 P1.2 / 0066 P2.2 (`l28_real_tcp_remove_member_left_on_disk`).
  - P2.1 `l28_tcp_leave_campaign_is_not_forall_traces`: `R-joint` and `R-swarm-real` stay continuous; campaign not a theorem.
  - P2.2 `l28_tcp_leave_verus_still_never`: `R-verus` stays in `never_floor`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0117 P2.2 CLI done; `residuals.json` `R-joint` owner 0118.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Planting a joint over TCP (needs add/remove member wire).
- Rocks coluna A/B. crates.io.
