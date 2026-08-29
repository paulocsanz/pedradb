# RFC: 0140 — In-process open loads raft peers from disk membership

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0139](0139-drop-preimages-local-non-member.md), [0125](0125-high-water-survives-open.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. RFC-0125 peeks disk voters **before** `load_range_peer` on the TCP ctor. In-process `open_with_envs_rng_opts` still builds every `RangePeer` from CLI `1..=n_nodes`, then `bind_cluster_identity` restores `ids`. Election timeout / preferred-leader stagger of a removed replica stays the 4-voter schedule. AS-IS `open_peer_uses_disk` is false. This slice: each node peeks its disk membership before load. 0139 drop-preimages is **not** this tooth. 0125 TCP peek is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `open_single_node` peeks `cluster_membership_key` then `load_range_peer`.
- `StoreCluster::open(n=4)` after leave still passes CLI `[1,2,3,4]` into `RangePeer::new`.

## Problems This Solves

- **Problem:** removed replica’s election timeout follows CLI n_nodes, not C-new.
- **Problem:** bind restores `ids` but does not rebuild the peer.
- **Problem:** AS-IS in-process open ignores disk at load.

## Proposed Solution

- Pure `open_peer_uses_disk(has_disk)` = `has_disk`. AS-IS false. `open_with_envs_rng_opts` peeks disk membership per node before `load_range_peer`. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (in-process open peek)
- [x] **P0.1** `open_peer_uses_disk` + load peek — status: `done`
- [x] **P0.2** Regression — status: `done` (`open_in_process_loads_disk_ids_before_peer`)

### P1 — next wave
- [x] **P1.1** lab Direct ctor — status: `done` (`open_lab_direct_loads_disk_ids_before_peer`)
- [x] **P1.2** TCP 3-process — status: `done` (`l28_real_tcp_removed_peer`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | in-process load from disk | done | open_with_envs_rng_opts | 2026-08-28 |
| P0.2 | p0 | removed replica timeout is C-new | done | open_in_process_loads_disk_ids_before_peer | 2026-08-28 |
| P1.1 | p1 | lab Direct ctor | done | open_lab_direct_loads_disk_ids_before_peer | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_peer | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + open_peer_disk | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | open_peer_uses_disk_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `open_peer_uses_disk(true)` true; AS-IS false. Same tokens raft=store.
  - `open_in_process_loads_disk_ids_before_peer`: Queued 4→3 leave; drop; `StoreCluster::open(n=4)`; node 4 `election_timeout` equals `election_timeout_for(4,1,[1,2,3])` not CLI-4. 0124 `!is_member(4)` is **not** this tooth. 0125 TCP peek is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_lab_direct_loads_disk_ids_before_peer`: same after `open_lab_direct`. production `open` is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_peer`: seed `0x0140_1E28` twice with `--remove-member`; fingerprints match; `peer=1`; production TCP ctor on the **removed** replica loads RangePeer from disk C-new (`election_timeout` equals `election_timeout_for(3,1,[1,2])`, not stale CLI `[1,2,3]`) and `!is_member`. 0125 high-water is **not** this tooth. 0139 drop-preimages is **not** this tooth. Exit via `l28_tcp_peer_ok`. AS-IS would keep CLI n_nodes at load. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `open_peer_disk` `entry: open_peer_uses_disk`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `open_peer_uses_disk_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `open_peer_uses_disk_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0139 P1.2 TCP drop-preimages is **done**; `local_node_id` HashMap first-key remains later; `residuals.json` `R-joint` owner 0140.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
  - `local_node_id` HashMap first-key (later).
