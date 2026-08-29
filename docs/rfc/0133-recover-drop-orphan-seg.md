# RFC: 0133 — Truncate persist must drop orphan log segments

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0132](0132-recover-truncate-local-non-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death**. RFC-0132 rewrites the log blob and caps `log_hi`. `persist_log_db` full rewrite **does not delete** `log_entry_key` rows at `index > new_hi`. A later `log_hi` bump (or a corrupt hi) reloads the uncommitted suffix. AS-IS `recover_drop_orphan_seg` is false. This slice: full rewrite deletes segments `i > last`. 0132 `log_hi` cap is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Incremental persist writes `log_entry_key(i)` and `log_hi`.
- Truncate persist rewrites the blob + `log_hi` and leaves the keys.

## Problems This Solves

- **Problem:** uncommitted suffix bytes remain on disk after F128 truncate.
- **Problem:** 0132 only moved the watermark.
- **Problem:** AS-IS never deletes orphan segments.

## Proposed Solution

- Pure `recover_drop_orphan_seg(seg_index, new_hi)` = `seg_index > new_hi`. AS-IS false. `persist_log_db` full rewrite deletes those keys. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (delete orphan keys)
- [x] **P0.1** `recover_drop_orphan_seg` + persist_log_db — status: `done`
- [x] **P0.2** Regression — status: `done` (`crash_reopen_drops_orphan_seg_on_removed_replica`)

### P1 — next wave
- [x] **P1.1** Process `open` of n=4 after leave — status: `done` (`open_drops_orphan_seg_on_removed_replica`)
- [x] **P1.2** TCP 3-process orphan drop — status: `done` (`l28_real_tcp_removed_orphan_drop`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | drop orphan segs | done | persist_log_db | 2026-08-28 |
| P0.2 | p0 | removed replica keys gone | done | crash_reopen_drops_orphan_seg_on_removed_replica | 2026-08-28 |
| P1.1 | p1 | process open n=4 | done | open_drops_orphan_seg_on_removed_replica | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_orphan_drop | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + recover_drop_orphan | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | recover_drop_orphan_seg_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `recover_drop_orphan_seg(3, 2)` true; AS-IS false. Same tokens raft=store.
  - `crash_reopen_drops_orphan_seg_on_removed_replica`: Queued 4→3 leave; plant durable uncommitted Put (writes `log_entry_key`); `crash_reopen_engine_on(4)`; `log_entry_key(commit+1)` is gone. 0132 `log_hi` cap is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_drops_orphan_seg_on_removed_replica`: same plant; drop; `StoreCluster::open(&dir, 4, 1)`; key gone. crash-reopen is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_orphan_drop`: seed `0x0133_1E28` twice with `--remove-member`; fingerprints match; `odrop=1`; plant a durable uncommitted Put (writes `log_entry_key`) on the **removed** replica then production TCP ctor deletes that key and `!is_member`. 0132 `log_hi` cap is **not** this tooth. Exit via `l28_tcp_odrop_ok`. AS-IS would leave the orphan segment. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `recover_drop_orphan` `entry: recover_drop_orphan_seg`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `recover_drop_orphan_seg_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `recover_drop_orphan_seg_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0134 P1.2 is TCP abort leftover 2PC on the removed replica; `residuals.json` `R-joint` owner 0135.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
