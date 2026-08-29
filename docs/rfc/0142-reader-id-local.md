# RFC: 0142 — LocalApplied reader id must be a local node

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0141](0141-local-id-if-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. After RFC-0141, TCP `local_node_id()` of a removed replica is None. `get()` still falls back to `ids.first()` (a **remote** voter) and `get_on` returns `bad node`. That is the wrong error class: we attempted a non-local read. AS-IS `reader_id_local` is true regardless of `is_local`. This slice: `ids.first()` counts only if that node is opened in this process. 0141 `local_node_id` is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `best_reader_for_key` ends with `ids.first()`.
- `get_fast_replica` / `keys_in_range_at` do the same.

## Problems This Solves

- **Problem:** removed replica `get()` tries a remote voter (`bad node`).
- **Problem:** follower-read path same fallback.
- **Problem:** AS-IS `ids.first()` ignores locality.

## Proposed Solution

- Pure `reader_id_local(is_local)` = `is_local`. AS-IS true. `ids_first_if_local` gates the fallback. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (`get` path)
- [x] **P0.1** `reader_id_local` + `ids_first_if_local` — status: `done`
- [x] **P0.2** Regression — status: `done` (`open_single_node_get_skips_remote_ids_first`)

### P1 — next wave
- [x] **P1.1** `get_fast_replica` same gate — status: `done` (`open_single_node_fast_replica_skips_remote_ids_first`)
- [x] **P1.2** TCP 3-process — status: `done` (`l28_real_tcp_removed_rdr`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | reader id must be local | done | ids_first_if_local | 2026-08-28 |
| P0.2 | p0 | TCP removed get is empty | done | open_single_node_get_skips_remote_ids_first | 2026-08-28 |
| P1.1 | p1 | fast replica same | done | open_single_node_fast_replica_skips_remote_ids_first | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_rdr | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + reader_local | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | reader_id_local_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `reader_id_local(false)` false; AS-IS true. Same tokens raft=store.
  - `open_single_node_get_skips_remote_ids_first`: Queued 4→3 leave; `open_single_node(4, stale CLI)`; `best_reader_for_key` is None; `get` Err contains `empty`, not `bad node`. 0141 `local_node_id` None is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_single_node_fast_replica_skips_remote_ids_first`: `get_fast_replica` Err contains `no replica`, not `bad node`. `get` is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_rdr`: seed `0x0142_1E28` twice with `--remove-member`; fingerprints match; `rdr=1`; production TCP ctor on the **removed** replica has `best_reader_for_key` None; `get` Err contains `empty`, not `bad node`; `!is_member`. 0141 `local_node_id` None is **not** this tooth. Exit via `l28_tcp_rdr_ok`. AS-IS would `get_on` a remote voter. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `reader_local` `entry: reader_id_local`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `reader_id_local_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `reader_id_local_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0141 P1.2 TCP local-id gate is **done**; `discard_uncommitted_from` ids-only remains later; `residuals.json` `R-joint` owner 0142.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
  - `discard_uncommitted_from` ids-only (later).
