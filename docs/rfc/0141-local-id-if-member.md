# RFC: 0141 — Sole local node id requires membership

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0140](0140-open-peer-uses-disk.md), [0128](0128-is-participating-requires-ids.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. `local_node_id` returns HashMap first-key when `nodes.len()==1`, **even if that node is not in `ids`**. After leave, the TCP ctor of the removed replica still answers `get()` from its local disk as cluster LocalApplied (stale keys the majority no longer owns). AS-IS `local_id_if_member` is true regardless of `ids`. This slice: the sole local id counts only if it is a current voter. 0140 in-process load peek is **not** this tooth. 0128 `is_participating` is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- TCP `open_single_node` has one local PedraDB.
- `get()` falls back to `local_node_id()` then `ids.first()`.

## Problems This Solves

- **Problem:** removed replica `get()` returns local-only bytes as cluster state.
- **Problem:** HashMap first-key ignores membership.
- **Problem:** AS-IS local id always counts when `len==1`.

## Proposed Solution

- Pure `local_id_if_member(in_ids)` = `in_ids`. AS-IS true. `local_node_id` ANDs the sole key with the kernel. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (TCP removed replica)
- [x] **P0.1** `local_id_if_member` + `local_node_id` gate — status: `done`
- [x] **P0.2** Regression — status: `done` (`open_single_node_local_id_omits_removed`)

### P1 — next wave
- [x] **P1.1** TCP ctor of a remaining member still has identity — status: `done` (`open_single_node_local_id_keeps_member`)
- [x] **P1.2** TCP 3-process — status: `done` (`l28_real_tcp_removed_lid`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | local id requires member | done | local_node_id | 2026-08-28 |
| P0.2 | p0 | TCP removed get fail-closed | done | open_single_node_local_id_omits_removed | 2026-08-28 |
| P1.1 | p1 | TCP member still has identity | done | open_single_node_local_id_keeps_member | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_lid | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + local_id_member | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | local_id_if_member_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `local_id_if_member(false)` false; AS-IS true. Same tokens raft=store.
  - `open_single_node_local_id_omits_removed`: Queued 4→3 leave; `open_single_node(4, stale CLI)`; `local_node_id()` is None; plant user key on node 4; `get` is Err (not `Ok(Some(stale))`). 0140 timeout peek is **not** this tooth. 0128 participating is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_single_node_local_id_keeps_member`: after leave, `open_single_node(1, stale CLI)`; `local_node_id()==Some(1)`. removed ctor is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_lid`: seed `0x0141_1E28` twice with `--remove-member`; fingerprints match; `lid=1`; production TCP ctor on the **removed** replica has `local_node_id()==None`; plant user key; `get` is Err (not `Ok(Some(stale))`) and `!is_member`. 0140 timeout peek is **not** this tooth. 0128 participating is **not** this tooth. Exit via `l28_tcp_lid_ok`. AS-IS would `get()` local-only bytes. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `local_id_member` `entry: local_id_if_member`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `local_id_if_member_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `local_id_if_member_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0140 P1.2 TCP disk-peer timeout is **done**; `get()` `ids.first()` fallback when the chosen id is not local remains later; `residuals.json` `R-joint` owner 0141.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
  - `get()` `ids.first()` fallback when the chosen id is not local (later).
  - `discard_uncommitted_from` ids-only (later).
