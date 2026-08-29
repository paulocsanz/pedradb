# RFC: 0137 — Persist abort fence on a local non-member

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0136](0136-persist-hist-local-non-member.md), [0134](0134-recover-abort-local-non-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. F47 `fence_txn_aborted` walks **current `ids`**. After leave, the removed replica never gets a durable abort fence; a later-committed orphan `TxnCommit` can materialise keys there. AS-IS `persist_fence_node_counts` requires `in_ids`. This slice: every **local** replica is fenced. 0136 SI hist is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `tx_cancel` / failed `tx_finish` call `fence_txn_aborted` so apply treats status `abort` as no-op.
- Fence iterates `ids`. TCP removed replica: no local voter.

## Problems This Solves

- **Problem:** removed replica has no abort fence.
- **Problem:** TCP removed process fence-put misses all replicas.
- **Problem:** AS-IS fence is ids-only.

## Proposed Solution

- Pure `persist_fence_node_counts(is_local, in_ids)` = `is_local`. AS-IS `is_local && in_ids`. `fence_txn_aborted` iterates local nodes. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (removed replica fence)
- [x] **P0.1** `persist_fence_node_counts` + fence loop — status: `done`
- [x] **P0.2** Regression — status: `done` (`fence_txn_aborted_on_removed_replica`)

### P1 — next wave
- [x] **P1.1** TCP ctor of the removed node — status: `done` (`open_single_node_fence_txn_when_removed`)
- [x] **P1.2** TCP 3-process fence — status: `done` (`l28_real_tcp_removed_fence`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | persist fence local | done | fence_txn_aborted | 2026-08-28 |
| P0.2 | p0 | removed replica fence | done | fence_txn_aborted_on_removed_replica | 2026-08-28 |
| P1.1 | p1 | TCP ctor removed | done | open_single_node_fence_txn_when_removed | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_fence | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + persist_fence | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | persist_fence_node_counts_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `persist_fence_node_counts(true, false)` true; AS-IS false. Same tokens raft=store.
  - `fence_txn_aborted_on_removed_replica`: Queued 4→3 leave; `fence_txn_aborted(tid)`; node 4 disk status is `abort`. 0136 hist is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_single_node_fence_txn_when_removed`: after leave, `open_single_node(4, stale CLI)`; fence; local disk is `abort`. in-process n=4 is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_fence`: seed `0x0137_1E28` twice with `--remove-member`; fingerprints match; `fence=1`; production TCP ctor on the **removed** replica persists abort fence on self and `!is_member`. 0136 SI hist and 0138 force-clear are **not** this tooth. Exit via `l28_tcp_fence_ok`. AS-IS would skip persist (`ids` filter). Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `persist_fence` `entry: persist_fence_node_counts`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `persist_fence_node_counts_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `persist_fence_node_counts_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0138 P1.2 is TCP force-local TX clear on the removed replica; `residuals.json` `R-joint` owner 0138.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
  - `force_local_clear_keys` / `drop_preimages` ids-only (later).
