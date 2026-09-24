# RFC: 0135 — Persist SI meta on a local non-member

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0134](0134-recover-abort-local-non-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. F56 `persist_u64_meta_all` (`now_ms` / generation / watermark) walks **current `ids`**. After leave, the removed replica never gets a durable clock; a TCP process whose only local node is the removed id fails every persist (no ids local). AS-IS `persist_meta_node_counts` requires `in_ids`. This slice: every **local** replica is written. 0134 leftover-abort is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `persist_now_ms` calls `persist_u64_meta_all("now_ms", …)`.
- Persist iterates `ids`. TCP removed replica: `nodes={self}`, `ids` is C-new.

## Problems This Solves

- **Problem:** removed replica's TTL clock is not durable.
- **Problem:** TCP removed process persist-now_ms fails all replicas.
- **Problem:** AS-IS persist is ids-only.

## Proposed Solution

- Pure `persist_meta_node_counts(is_local, in_ids)` = `is_local`. AS-IS `is_local && in_ids`. `persist_u64_meta_all` iterates local nodes. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (removed replica now_ms)
- [x] **P0.1** `persist_meta_node_counts` + persist loop — status: `done`
- [x] **P0.2** Regression — status: `done` (`persist_now_ms_on_removed_replica`)

### P1 — next wave
- [x] **P1.1** TCP ctor of the removed node — status: `done` (`open_single_node_persist_now_ms_when_removed`)
- [x] **P1.2** TCP 3-process now_ms — status: `done` (`l28_real_tcp_removed_now_ms`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | persist meta local | done | persist_u64_meta_all | 2026-08-28 |
| P0.2 | p0 | removed replica now_ms | done | persist_now_ms_on_removed_replica | 2026-08-28 |
| P1.1 | p1 | TCP ctor removed | done | open_single_node_persist_now_ms_when_removed | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_now_ms | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + persist_meta | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | persist_meta_node_counts_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `persist_meta_node_counts(true, false)` true; AS-IS false. Same tokens raft=store.
  - `persist_now_ms_on_removed_replica`: Queued 4→3 leave; `advance_now_ms`; node 4 disk `now_ms` equals RAM. 0134 abort is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_single_node_persist_now_ms_when_removed`: after leave, `open_single_node(4, stale CLI)`; `advance_now_ms`; local disk `now_ms` equals RAM. in-process n=4 is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_now_ms`: seed `0x0135_1E28` twice with `--remove-member`; fingerprints match; `nowms=1`; production TCP ctor on the **removed** replica persists `now_ms` on self and `!is_member`. 0134 abort and 0136 hist are **not** this tooth. Exit via `l28_tcp_nowms_ok`. AS-IS would skip persist (`ids` filter). Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `persist_meta` `entry: persist_meta_node_counts`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `persist_meta_node_counts_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `persist_meta_node_counts_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0136 P1.2 is TCP persist SI hist on the removed replica; `residuals.json` `R-joint` owner 0136.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
