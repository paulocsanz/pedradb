# RFC: 0125 — High-water and disk membership must be used **before** loading the raft log

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0124](0124-disk-membership-overrides-cli.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. RFC-0124 restores `ids` in `bind_cluster_identity` **after** `load_range_peer` already built the peer from CLI `--peer`. `membership_high_water` is RAM-only: restart with honest C-new CLI (`n=3` after a 4-node shrink) sets high-water to 3, so out-of-band `remove_member` can pass the quorum floor that 4-node history forbids. AS-IS `high_water_at_least` returns RAM only. This slice: persist high-water with identity; apply disk voters **before** `load_range_peer`; restore high-water on bind/open. 0124 bind-after-load is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Quorum floor: `2·(⌊next/2⌋+1) > high_water`.
- `persist_cluster_identity` writes ids, not high-water.
- `open_single_node` loads the log with CLI ids, then bind.

## Problems This Solves

- **Problem:** restart with C-new CLI forgets high-water 4.
- **Problem:** TCP open builds `RangePeer` from stale `--peer`.
- **Problem:** AS-IS RAM/CLI high-water only.

## Proposed Solution

- Pure `high_water_at_least(disk, ram) = max`. AS-IS = `ram`. Persist `high_water` next to membership. Peek disk voters before `load_range_peer`. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (high-water + load order)
- [x] **P0.1** Persist/restore high-water; open loads disk voters first — status: `done`
- [x] **P0.2** Regression — status: `done` (`high_water_survives_reopen_refuses_oob_shrink`)

### P1 — next wave
- [x] **P1.1** `open_single_node` stale CLI after leave — status: `done` (`open_single_node_stale_cli_loads_disk_membership`)
- [x] **P1.2** Verus twin of `disk_membership_overrides_cli` / `high_water_at_least` — status: `done` (catalog `disk_membership` 0124 + `high_water`)

### P2 — later
- [x] **P2.1** Campaign is not ∀ traces — status: `done` (`high_water_at_least_campaign_is_not_forall_traces`)
- [x] **P2.2** R-verus still never — status: `done` (`high_water_at_least_verus_still_never`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | persist high-water + load order | done | persist_cluster_identity + open_single_node | 2026-08-28 |
| P0.2 | p0 | reopen keeps quorum floor | done | high_water_survives_reopen_refuses_oob_shrink | 2026-08-28 |
| P1.1 | p1 | TCP ctor stale CLI | done | open_single_node_stale_cli_loads_disk_membership | 2026-08-28 |
| P1.2 | p1 | Verus twin | done | catalog high_water + membership_joint.rs | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | done | high_water_at_least_campaign_is_not_forall_traces | 2026-08-28 |
| P2.2 | p2 | R-verus never | done | high_water_at_least_verus_still_never | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `high_water_at_least(4, 3) == 4`; AS-IS returns 3.
  - `high_water_survives_reopen_refuses_oob_shrink`: 4-node Queued shrink+leave; drop; reopen 3 nodes on the same dirs; `remove_member` of a remaining voter is quorum-floor Err. AS-IS high-water 3 would allow. Runs on Darwin. Does not submit io_uring SQEs.
  - `open_single_node_stale_cli_loads_disk_membership`: after leave, `open_single_node(self=1, CLI=[1,2,3,4])` is `!is_member(4)`. 0124 bind-after-load is **not** this tooth.
  - P1.2 catalog pair `high_water` `entry: high_water_at_least`; twin `membership_joint.rs` (`disk_membership_overrides_cli` already 0124 P2.1). Freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.1 `high_water_at_least_campaign_is_not_forall_traces`: `R-joint` stays continuous; catalog pair `high_water`; campaign not a theorem.
  - P2.2 `high_water_at_least_verus_still_never`: `R-verus` stays in `never_floor`; catalog/twin freeze of `high_water_at_least` is not a verified verifier. Does not run `verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0124 P2.1 stays Verus; `residuals.json` `R-joint` owner 0125.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
