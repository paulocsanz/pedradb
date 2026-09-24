# RFC: 0138 — Force-local TX clear on a local non-member

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0137](0137-persist-fence-local-non-member.md), [0134](0134-recover-abort-local-non-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. I-TX-2 `force_local_clear_keys` walks **current `ids`**. After leave, the removed replica keeps stuck intents (`Conflict` forever). RFC-0134 abort leftover is recover-only; live `tx_cancel` still misses that replica. AS-IS `force_clear_node_counts` requires `in_ids`. This slice: every **local** replica is cleared. 0137 abort fence is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `tx_cancel` fences then force-local abort/revert when a range has no leader.
- Clear iterates `ids`. TCP removed replica: no local voter.

## Problems This Solves

- **Problem:** removed replica keeps durable stuck intents.
- **Problem:** TCP removed process clear misses all replicas.
- **Problem:** AS-IS clear is ids-only.

## Proposed Solution

- Pure `force_clear_node_counts(is_local, in_ids)` = `is_local`. AS-IS `is_local && in_ids`. `force_local_clear_keys` iterates local nodes. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (removed replica clear)
- [x] **P0.1** `force_clear_node_counts` + clear loop — status: `done`
- [x] **P0.2** Regression — status: `done` (`force_local_clear_on_removed_replica`)

### P1 — next wave
- [x] **P1.1** TCP ctor of the removed node — status: `done` (`open_single_node_force_clear_when_removed`)
- [x] **P1.2** TCP 3-process clear — status: `done` (`l28_real_tcp_removed_clear`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | force-clear local | done | force_local_clear_keys | 2026-08-28 |
| P0.2 | p0 | removed replica intent gone | done | force_local_clear_on_removed_replica | 2026-08-28 |
| P1.1 | p1 | TCP ctor removed | done | open_single_node_force_clear_when_removed | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_clear | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + force_clear | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | force_clear_node_counts_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `force_clear_node_counts(true, false)` true; AS-IS false. Same tokens raft=store.
  - `force_local_clear_on_removed_replica`: Queued 4→3 leave; plant intent on node 4; `force_local_clear_keys` abort; `intent_key` gone. 0137 fence is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_single_node_force_clear_when_removed`: after leave, `open_single_node(4, stale CLI)`; plant + clear; local intent gone. in-process n=4 is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_clear`: seed `0x0138_1E28` twice with `--remove-member`; fingerprints match; `clear=1`; production TCP ctor on the **removed** replica plants a stuck intent then force-clears it (`intent_key` gone) and `!is_member`. 0137 abort fence is **not** this tooth. Exit via `l28_tcp_clear_ok`. AS-IS would skip clear (`ids` filter). Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `force_clear` `entry: force_clear_node_counts`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `force_clear_node_counts_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `force_clear_node_counts_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0139 P1.2 TCP drop-preimages is **done**; `residuals.json` `R-joint` owner 0139.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
  - `drop_preimages` ids-only (later).
