# RFC: 0131 — Recover must apply on a local non-member replica

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0130](0130-recover-apply-committed.md), [0128](0128-is-participating-requires-ids.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. RFC-0130 walks `recover_apply_committed` over **current `ids`**. After a leave apply, C-new omits the removed replica; that process still holds a local PedraDB whose log can have `commit > applied`. Recover skips it. AS-IS `recover_apply_node_counts` requires `in_ids`. This slice: every **local** replica is eligible; membership is not a recover filter. 0130 apply-on-voters is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `recover_apply_committed` clones `self.ids` then `apply_range` when `commit > applied`.
- After shrink, the removed node stays in `nodes` (in-process) / is the only local (TCP).
- Live apply already skips it (`is_participating`); recover must still catch the durable prefix.

## Problems This Solves

- **Problem:** crash on a removed replica leaves committed log unapplied.
- **Problem:** 0130 only recovers current voters.
- **Problem:** AS-IS filters recover by `ids`.

## Proposed Solution

- Pure `recover_apply_node_counts(is_local, in_ids)` = `is_local`. AS-IS `is_local && in_ids`. `recover_apply_committed` iterates local nodes. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (removed replica recover)
- [x] **P0.1** `recover_apply_node_counts` + recover loop — status: `done`
- [x] **P0.2** Regression — status: `done` (`crash_reopen_applies_committed_on_removed_replica`)

### P1 — next wave
- [x] **P1.1** Process `open` of n=4 after leave — status: `done` (`open_applies_committed_on_removed_replica`)
- [x] **P1.2** TCP 3-process removed replica — status: `done` (`l28_real_tcp_removed_recover_apply`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | recover local not ids | done | recover_apply_committed | 2026-08-28 |
| P0.2 | p0 | removed replica crash-reopen | done | crash_reopen_applies_committed_on_removed_replica | 2026-08-28 |
| P1.1 | p1 | process open n=4 | done | open_applies_committed_on_removed_replica | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_recover_apply | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + recover_apply_node | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | recover_apply_node_counts_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `recover_apply_node_counts(true, false)` true; AS-IS false. Same tokens raft=store.
  - `crash_reopen_applies_committed_on_removed_replica`: Queued 4→3 leave; plant durable committed Put on node 4 **without** apply; `crash_reopen_engine_on(4)`; `get_on(4, key)` is the value and applied ≥ commit. 0130 leader-in-ids plant is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_applies_committed_on_removed_replica`: same plant; drop; `StoreCluster::open(&dir, 4, 1)`; `get_on(4, key)` is the value. crash-reopen is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_recover_apply`: seed `0x0131_1E28` twice with `--remove-member`; fingerprints match; `napply=1`; plant a durable committed-unapplied Noop on the **removed** replica's Pedra dir then production TCP ctor closes `recover_must_apply` and `!is_member`. 0130 remaining-voter apply is **not** this tooth. Exit via `l28_tcp_napply_ok`. AS-IS would skip apply (`ids` filter). Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `recover_apply_node` `entry: recover_apply_node_counts`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `recover_apply_node_counts_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `recover_apply_node_counts_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0132 P1.2 is TCP truncate on the removed replica; `residuals.json` `R-joint` owner 0133.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
