# RFC: 0126 — `crash_reopen` must restore durable high-water

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0125](0125-high-water-survives-open.md), [0124](0124-disk-membership-overrides-cli.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. RFC-0125 persists `high_water` and restores it on `bind`/`open`. `crash_reopen_engine_on` / `reopen_engine_on` reload `ids` then set `high_water = max(ram, ids.len())` — they **never read** the durable key. A RAM high-water that was forgotten (3 after a 4-node shrink) stays 3 across crash-reopen; OOB `remove_member` can pass the floor. AS-IS `high_water_at_least` returns RAM. This slice: both reopen paths apply `high_water_at_least(disk, ram)`. Verus twin + catalog pair. 0125 process-open is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Durable key `high_water` sits next to membership.
- Crash-reopen reloads voters, not high-water.

## Problems This Solves

- **Problem:** crash-reopen forgets 4-node history.
- **Problem:** Verus twin of `high_water_at_least` missing.
- **Problem:** AS-IS RAM high-water only.

## Proposed Solution

- Reopen paths call `high_water_at_least(disk, ram)`. Twin exec + catalog `high_water`. Named crash-reopen floor test. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (reopen reads disk high-water)
- [x] **P0.1** crash/reopen restore high-water — status: `done`
- [x] **P0.2** Regression — status: `done` (`crash_reopen_restores_high_water`)

### P1 — next wave
- [x] **P1.1** Verus twin + catalog pair `high_water` — status: `done`
- [x] **P1.2** 3-process high-water on TCP restart — status: `done` (`l28_real_tcp_high_water_after_remove`)

### P2 — later
- [x] **P2.1** Campaign is not ∀ traces — status: `done` (`l28_tcp_hw_campaign_is_not_forall_traces`)
- [x] **P2.2** R-verus still never — status: `done` (`l28_tcp_hw_verus_still_never`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | reopen restores high-water | done | crash_reopen_engine_on + reopen_engine_on | 2026-08-28 |
| P0.2 | p0 | forgotten RAM high-water | done | crash_reopen_restores_high_water | 2026-08-28 |
| P1.1 | p1 | Verus twin | done | membership_joint.rs + catalog high_water | 2026-08-28 |
| P1.2 | p1 | TCP process high-water | done | l28_real_tcp_high_water_after_remove | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | done | l28_tcp_hw_campaign_is_not_forall_traces | 2026-08-28 |
| P2.2 | p2 | R-verus never | done | l28_tcp_hw_verus_still_never | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `high_water_at_least(4, 3) == 4`; AS-IS 3.
  - `crash_reopen_restores_high_water`: Queued 4→3 leave; persist; RAM high-water forced to 3; `crash_reopen_engine_on` leader; `remove_member` of a remaining voter is quorum-floor Err. 0125 process-open is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - Catalog pair `high_water` `entry: high_water_at_least`; freeze twins fail if the exec fn is dropped.
  - P1.2 `l28_real_tcp_high_water_after_remove`: seed `0x0126_1E28` twice with `--remove-member`; fingerprints match; `hw=1`; `high_water_at_least(disk, 2) >= 3`; exit via `l28_tcp_hw_ok`. AS-IS would skip the scan. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 `l28_tcp_hw_campaign_is_not_forall_traces`: `R-joint` and `R-swarm-real` stay continuous; catalog pair `l28_tcp_hw`; campaign not a theorem.
  - P2.2 `l28_tcp_hw_verus_still_never`: `R-verus` stays in `never_floor`; catalog/twin freeze of `l28_tcp_hw_ok` is not a verified verifier. Does not run `verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0125 P1.2 done; `residuals.json` `R-joint` owner 0126.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
