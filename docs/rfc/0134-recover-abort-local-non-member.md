# RFC: 0134 — Recover must abort leftover 2PC on a local non-member

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0132](0132-recover-truncate-local-non-member.md), [0131](0131-recover-apply-local-non-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. F35 `abort_leftover_intents` walks **current `ids`**. After leave, the removed replica still holds prepared intents; recover skips it. `crash_reopen_engine_on` never calls abort. AS-IS `recover_abort_node_counts` requires `in_ids`. This slice: every **local** replica is aborted; crash-reopen aborts. 0132 log truncate is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Open recovery aborts leftover prepared TX (no coordinator log).
- Abort iterates `ids`. Crash-reopen stops at truncate + apply.

## Problems This Solves

- **Problem:** removed replica keeps immortal intents across reopen.
- **Problem:** crash-reopen never runs F35 abort.
- **Problem:** AS-IS abort is ids-only.

## Proposed Solution

- Pure `recover_abort_node_counts(is_local, in_ids)` = `is_local`. AS-IS `is_local && in_ids`. `abort_leftover_intents` iterates local nodes; crash/reopen call it. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (removed replica abort)
- [x] **P0.1** `recover_abort_node_counts` + abort loop + crash-reopen — status: `done`
- [x] **P0.2** Regression — status: `done` (`crash_reopen_aborts_leftover_on_removed_replica`)

### P1 — next wave
- [x] **P1.1** Process `open` of n=4 after leave — status: `done` (`open_aborts_leftover_on_removed_replica`)
- [x] **P1.2** TCP 3-process abort — status: `done` (`l28_real_tcp_removed_abort`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | abort leftover local | done | abort_leftover_intents + crash_reopen | 2026-08-28 |
| P0.2 | p0 | removed replica intent gone | done | crash_reopen_aborts_leftover_on_removed_replica | 2026-08-28 |
| P1.1 | p1 | process open n=4 | done | open_aborts_leftover_on_removed_replica | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_abort | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + recover_abort | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | recover_abort_node_counts_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `recover_abort_node_counts(true, false)` true; AS-IS false. Same tokens raft=store.
  - `crash_reopen_aborts_leftover_on_removed_replica`: Queued 4→3 leave; plant durable leftover intent on node 4; `crash_reopen_engine_on(4)`; `intent_key` is gone. 0132 truncate is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_aborts_leftover_on_removed_replica`: same plant; drop; `StoreCluster::open(&dir, 4, 1)`; intent gone. crash-reopen is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_abort`: seed `0x0134_1E28` twice with `--remove-member`; fingerprints match; `abort=1`; plant a leftover 2PC intent on the **removed** replica then production TCP ctor deletes `intent_key` and `!is_member`. 0133 orphan drop is **not** this tooth. Exit via `l28_tcp_abort_ok`. AS-IS would skip abort (`ids` filter). Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `recover_abort` `entry: recover_abort_node_counts`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `recover_abort_node_counts_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `recover_abort_node_counts_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0135 P1.2 is TCP persist now_ms on the removed replica; `residuals.json` `R-joint` owner 0135.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
