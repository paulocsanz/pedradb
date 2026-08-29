# RFC: 0139 — Drop TX preimages on a local non-member

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0138](0138-force-clear-local-non-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. After a successful all-range commit, `drop_preimages` walks **current `ids`**. After leave, the removed replica keeps prepare-time preimages; a later revert can restore a stale value there. AS-IS `drop_preimages_node_counts` requires `in_ids`. This slice: every **local** replica drops preimages. 0138 force-clear is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `tx_finish` records SI then drops preimages so leftover pre cannot confuse later reverts.
- Drop iterates `ids`. TCP removed replica: no local voter.

## Problems This Solves

- **Problem:** removed replica keeps durable prepare-time preimages.
- **Problem:** TCP removed process drop misses all replicas.
- **Problem:** AS-IS drop is ids-only.

## Proposed Solution

- Pure `drop_preimages_node_counts(is_local, in_ids)` = `is_local`. AS-IS `is_local && in_ids`. `drop_preimages` iterates local nodes. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (removed replica drop)
- [x] **P0.1** `drop_preimages_node_counts` + drop loop — status: `done`
- [x] **P0.2** Regression — status: `done` (`drop_preimages_on_removed_replica`)

### P1 — next wave
- [x] **P1.1** TCP ctor of the removed node — status: `done` (`open_single_node_drop_preimages_when_removed`)
- [x] **P1.2** TCP 3-process drop — status: `done` (`l28_real_tcp_removed_pre`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | drop-preimages local | done | drop_preimages | 2026-08-28 |
| P0.2 | p0 | removed replica pre gone | done | drop_preimages_on_removed_replica | 2026-08-28 |
| P1.1 | p1 | TCP ctor removed | done | open_single_node_drop_preimages_when_removed | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_pre | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + drop_preimages | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | drop_preimages_node_counts_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `drop_preimages_node_counts(true, false)` true; AS-IS false. Same tokens raft=store.
  - `drop_preimages_on_removed_replica`: Queued 4→3 leave; plant preimage on node 4; `drop_preimages`; `txn_pre_key` gone. 0138 force-clear is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_single_node_drop_preimages_when_removed`: after leave, `open_single_node(4, stale CLI)`; plant + drop; local preimage gone. in-process n=4 is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_pre`: seed `0x0139_1E28` twice with `--remove-member`; fingerprints match; `pre=1`; production TCP ctor on the **removed** replica plants a leftover preimage then drops it (`txn_pre_key` gone) and `!is_member`. 0138 force-clear is **not** this tooth. Exit via `l28_tcp_pre_ok`. AS-IS would skip drop (`ids` filter). Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `drop_preimages` `entry: drop_preimages_node_counts`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `drop_preimages_node_counts_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `drop_preimages_node_counts_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; `open_with_envs` CLI `load_range_peer` before bind and `local_node_id` HashMap first-key remain later; `residuals.json` `R-joint` owner 0139.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
  - `open_with_envs` CLI `load_range_peer` before bind (later).
  - `local_node_id` HashMap first-key (later).
