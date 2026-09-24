# RFC: 0070 — PCT depth is not ∀ OS schedules (fail-closed)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0051](0051-beyond-fdb-sim-holes.md), [0057](0057-maximum-intensity-parallel-dst-boxes-formal.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-pct` (not `never_floor`). Axis vs FDB Sim2: **G1 client/OS threads**. FDB Sim2 is single-threaded Flow; their client tester is not seed-replayable. Pedra `ConcurrentDb` *is* OS threads + group commit. PCT d=2 finds planted bugs; it does **not** cover ∀ interleavings. This slice closes the rounding: a campaign of finite PCT depth cannot be claimed as “serial==parallel for all schedules.”

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. PCT remains a campaign. TSan + swarm stay the other teeth. `∀π` of `ConcurrentDb` stays TCB (R-group-glue).

## Background

- RFC-0051: PCT d=2 finds the planted depth-2 atomicity bug; sequential/round-robin miss it. Close-text of `R-pct` already said “not forall schedules.”
- Production `ConcurrentDb` has no API that *names* that a green PCT run is not ∀π. A d=2 CLEAN can be rounded to “we covered interleavings.”
- FDB G1: they published the hole. Pedra has a stronger tester (PCT+replay) and still must not round it to a theorem.

## Problems This Solves

- **Problem:** finite PCT depth looks like a ∀-schedules proof if nobody fail-closes the claim.
- **Problem:** the honesty lived only in RFC/residual text, not a kernel the live `ConcurrentDb` calls.
- **Problem:** AS-IS treats `pct_depth >= 2` as forall.

## Proposed Solution

- Pure `forall_schedules_admitted(pct_depth)` always `false`. AS-IS `pct_depth >= 2`.
- Lives in existing `group_commit_kernel` (no new TCB file). Production `ConcurrentDb::claim_forall_schedules` calls it after a real open/put.

## Delivery slices (mandatory)

### P0 — must ship first (claim on the live ConcurrentDb path)
- [x] **P0.1** `group_commit_kernel::forall_schedules_admitted` + AS-IS — status: `done`
- [x] **P0.2** `ConcurrentDb::claim_forall_schedules` on the live engine — status: `done`
- [x] **P0.3** Regression: open+put then d=2 claim is false; AS-IS would admit — status: `done` (`claim_forall_schedules_refused_at_pct_depth2`)

### P1 — next wave
- [x] **P1.1** World/PCT runner refuses a forall flag unless the kernel admits — status: `done` (`world_pct_run_refuses_forall_schedules`; `pct_runner_refuses_forall_schedules_at_depth2`)
- [x] **P1.2** swarm `serial==parallel` gate documents it is not ∀π — status: `done` (`serial_parallel_is_forall`; `world_swarm_parallel_matches_serial`)

### P2 — later
- [x] **P2.1** Verus twin of `forall_schedules_admitted` — status: `done` (`verus/group_commit.rs` + catalog `forall_schedules`)
- [x] **P2.2** PCT d>2 campaign remains 0051; this RFC does not raise default depth — status: `done` (`pct_campaign_default_depth`; `claim_default_pct_depth_not_raised`; `world_pct_default_depth_not_raised`; `pct_runner_default_depth_not_raised`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | forall_schedules_admitted kernel + AS-IS | done | group_commit_kernel.rs | 2026-08-27 |
| P0.2 | p0 | claim_forall_schedules on ConcurrentDb | done | concurrent.rs | 2026-08-27 |
| P0.3 | p0 | d=2 claim tooth | done | claim_forall_schedules_refused_at_pct_depth2 | 2026-08-27 |
| P1.1 | p1 | PCT runner uses kernel | done | world_pct_run_refuses_forall_schedules + pct_runner_refuses_forall_schedules_at_depth2 | 2026-08-27 |
| P1.2 | p1 | swarm serial=parallel honesty | done | serial_parallel_is_forall + world_swarm_parallel_matches_serial | 2026-08-27 |
| P2.1 | p2 | Verus twin | done | group_commit.rs forall_schedules_admitted + catalog forall_schedules | 2026-08-28 |
| P2.2 | p2 | do not raise default PCT depth | done | claim_default_pct_depth_not_raised + world_pct_default_depth_not_raised + pct_runner_default_depth_not_raised | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `forall_schedules_admitted(2)` is false; `forall_schedules_admitted_as_is(2)` is true.
  - `claim_forall_schedules_refused_at_pct_depth2`: `ConcurrentDb::open`, `put`, `get` sees the value, then `claim_forall_schedules(2)` is false.
  - P1.1 `world_pct_run_refuses_forall_schedules`: live `World::run` with `node_step_pct`; `claim_forall_schedules` false; AS-IS d=2 would admit. `pct_runner_refuses_forall_schedules_at_depth2`: live `run_pcts` d=2, same.
  - P1.2 `world_swarm_parallel_matches_serial`: hashes still match; `serial_parallel_is_forall(true)` is false; AS-IS would admit.
  - P2.1 catalog pair `forall_schedules` entry `forall_schedules_admitted` with Verus twin in `verus/group_commit.rs` (freeze of twin files; `verus` not on PATH).
  - P2.2 `claim_default_pct_depth_not_raised`: live ConcurrentDb open/put; `pct_campaign_default_depth()` is 2; `claim_default_pct_depth_raised` is false; AS-IS `default_pct_depth_raised_as_is` would admit. `world_pct_default_depth_not_raised` and `pct_runner_default_depth_not_raised` (`PiPolicy::pct_campaign_default`) same. d>2 remains RFC-0051 `planted_depth3_three_teeth`.
- **Telemetry / Analytics:** none — honesty invariant.
- **Documentation:** this RFC; `residuals.json` `R-pct` close-text + owner 0070.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Proving ∀ OS interleavings (R-group-glue). TSan as a theorem. Raising default PCT depth.
- Extracting `db.rs`. Flow. `never_floor`. Rocks coluna A/B. crates.io.
- RFC-0069 ES axioms (different residual).
