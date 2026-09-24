# RFC: 0136 — Persist SI hist on a local non-member

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0135](0135-persist-meta-local-non-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. RFC-0135 writes u64 SI meta on every local replica. `persist_si_keys` still walks **`ids` ∩ participating** for hist rows. After leave, the removed replica never gets a durable SI hist mirror; a TCP process whose only local node is the removed id writes none. AS-IS `persist_hist_node_counts` requires `in_ids`. This slice: every **local** replica is written. 0135 `now_ms` is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `note_mutations_at` calls `persist_si_keys` to mirror hist.
- Hist loop iterates `ids` and skips `!is_participating`.

## Problems This Solves

- **Problem:** removed replica's SI hist is not durable.
- **Problem:** TCP removed process writes zero hist rows.
- **Problem:** AS-IS hist persist is ids-only.

## Proposed Solution

- Pure `persist_hist_node_counts(is_local, in_ids)` = `is_local`. AS-IS `is_local && in_ids`. `persist_si_keys` iterates local nodes. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (removed replica hist)
- [x] **P0.1** `persist_hist_node_counts` + hist loop — status: `done`
- [x] **P0.2** Regression — status: `done` (`persist_si_hist_on_removed_replica`)

### P1 — next wave
- [x] **P1.1** TCP ctor of the removed node — status: `done` (`open_single_node_persist_si_hist_when_removed`)
- [x] **P1.2** TCP 3-process hist — status: `done` (`l28_real_tcp_removed_hist`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | persist hist local | done | persist_si_keys | 2026-08-28 |
| P0.2 | p0 | removed replica hist | done | persist_si_hist_on_removed_replica | 2026-08-28 |
| P1.1 | p1 | TCP ctor removed | done | open_single_node_persist_si_hist_when_removed | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_hist | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + persist_hist | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | persist_hist_node_counts_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `persist_hist_node_counts(true, false)` true; AS-IS false. Same tokens raft=store.
  - `persist_si_hist_on_removed_replica`: Queued 4→3 leave; plant RAM hist; `persist_si_keys`; node 4 disk has `hist_key`. 0135 now_ms is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_single_node_persist_si_hist_when_removed`: after leave, `open_single_node(4, stale CLI)`; persist hist; local disk has the key. in-process n=4 is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_hist`: seed `0x0136_1E28` twice with `--remove-member`; fingerprints match; `hist=1`; production TCP ctor on the **removed** replica persists SI hist on self and `!is_member`. 0135 now_ms and 0137 abort fence are **not** this tooth. Exit via `l28_tcp_hist_ok`. AS-IS would skip persist (`ids` filter). Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `persist_hist` `entry: persist_hist_node_counts`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `persist_hist_node_counts_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `persist_hist_node_counts_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0137 P1.2 is TCP persist abort fence on the removed replica; `residuals.json` `R-joint` owner 0137.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
