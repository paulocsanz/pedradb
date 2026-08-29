# RFC: 0132 — Recover must persist truncated logs on a local non-member

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0131](0131-recover-apply-local-non-member.md), [0130](0130-recover-apply-committed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. `load_range_peer` drops the uncommitted suffix in RAM (I-MAJ-3 / F128). `persist_truncated_logs` writes that truncation to disk, but walks **current `ids`**. After leave, the removed replica's Pedra still holds `index > commit` on disk. `crash_reopen_engine_on` never calls the persist at all. AS-IS `recover_truncate_node_counts` requires `in_ids`. This slice: every **local** replica is truncated to disk; crash-reopen persists. 0131 recover-apply is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Open loads the log, then `retain index ≤ commit`, then persist.
- Persist iterates `ids`. Crash-reopen stops at RAM load + 0131 apply.

## Problems This Solves

- **Problem:** removed replica keeps an uncommitted suffix on disk across reopen.
- **Problem:** crash-reopen never rewrites the truncated log.
- **Problem:** AS-IS persist is ids-only.

## Proposed Solution

- Pure `recover_truncate_node_counts(is_local, in_ids)` = `is_local`. AS-IS `is_local && in_ids`. `persist_truncated_logs` iterates local nodes; crash/reopen call it. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (removed replica disk truncate)
- [x] **P0.1** `recover_truncate_node_counts` + persist loop + crash-reopen — status: `done`
- [x] **P0.2** Regression — status: `done` (`crash_reopen_truncates_uncommitted_on_removed_replica`)

### P1 — next wave
- [x] **P1.1** Process `open` of n=4 after leave — status: `done` (`open_truncates_uncommitted_on_removed_replica`)
- [x] **P1.2** TCP 3-process truncate — status: `done` (`l28_real_tcp_removed_truncate`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | persist truncate local | done | persist_truncated_logs + crash_reopen | 2026-08-28 |
| P0.2 | p0 | removed replica disk suffix | done | crash_reopen_truncates_uncommitted_on_removed_replica | 2026-08-28 |
| P1.1 | p1 | process open n=4 | done | open_truncates_uncommitted_on_removed_replica | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_truncate | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + recover_truncate | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | recover_truncate_node_counts_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `recover_truncate_node_counts(true, false)` true; AS-IS false. Same tokens raft=store.
  - `crash_reopen_truncates_uncommitted_on_removed_replica`: Queued 4→3 leave; plant durable **uncommitted** Put on node 4; `crash_reopen_engine_on(4)`; disk log (and `log_hi`) has no `index > commit`. 0131 applied-put is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_truncates_uncommitted_on_removed_replica`: same plant; drop; `StoreCluster::open(&dir, 4, 1)`; disk log on node 4 has no suffix. crash-reopen is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_truncate`: seed `0x0132_1E28` twice with `--remove-member`; fingerprints match; `trunc=1`; plant a durable uncommitted Put on the **removed** replica's Pedra dir then production TCP ctor persists truncate (`log_hi`/`log` have no `index > commit`) and `!is_member`. 0131 apply and 0133 orphan-key drop are **not** this tooth. Exit via `l28_tcp_trunc_ok`. AS-IS would skip persist (`ids` filter). Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `recover_truncate` `entry: recover_truncate_node_counts`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `recover_truncate_node_counts_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `recover_truncate_node_counts_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0133 P1.2 is TCP orphan-segment drop; `residuals.json` `R-joint` owner 0134.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
