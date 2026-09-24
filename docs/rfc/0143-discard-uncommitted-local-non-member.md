# RFC: 0143 — Live uncommitted-log discard on a local non-member

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0142](0142-reader-id-local.md), [0132](0132-recover-truncate-local-non-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. `discard_uncommitted_from` walks **current `ids`**. After leave, a NotCommitted abort leaves the uncommitted suffix on the removed replica’s RAM log (and disk if persist ran). RFC-0132 only truncates on recover. AS-IS `discard_node_counts` requires `in_ids`. This slice: every **local** replica is discarded. 0142 reader locality is **not** this tooth. 0132 crash-reopen truncate is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Client propose that fails majority calls `discard_uncommitted_from`.
- Loop is `for nid in ids`. TCP removed replica: no local voter.

## Problems This Solves

- **Problem:** removed replica keeps an uncommitted log suffix after abort.
- **Problem:** reopen can resurrect that suffix if persist also skipped.
- **Problem:** AS-IS discard is ids-only.

## Proposed Solution

- Pure `discard_node_counts(is_local, in_ids)` = `is_local`. AS-IS `is_local && in_ids`. `discard_uncommitted_from` iterates local nodes. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (removed replica discard)
- [x] **P0.1** `discard_node_counts` + discard loop — status: `done`
- [x] **P0.2** Regression — status: `done` (`discard_uncommitted_on_removed_replica`)

### P1 — next wave
- [x] **P1.1** TCP ctor of the removed node — status: `done` (`open_single_node_discard_uncommitted_when_removed`)
- [x] **P1.2** TCP 3-process — status: `done` (`l28_real_tcp_removed_dsc`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | discard local | done | discard_uncommitted_from | 2026-08-28 |
| P0.2 | p0 | removed replica suffix gone | done | discard_uncommitted_on_removed_replica | 2026-08-28 |
| P1.1 | p1 | TCP ctor removed | done | open_single_node_discard_uncommitted_when_removed | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_removed_dsc | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + discard_uncommitted | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | discard_node_counts_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `discard_node_counts(true, false)` true; AS-IS false. Same tokens raft=store.
  - `discard_uncommitted_on_removed_replica`: Queued 4→3 leave; plant uncommitted suffix on node 4; `discard_uncommitted_from`; RAM and disk suffix gone. 0132 recover truncate is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_single_node_discard_uncommitted_when_removed`: after leave, `open_single_node(4, stale CLI)`; plant + discard; local suffix gone. in-process n=4 is **not** this tooth.
  - P1.2 `l28_real_tcp_removed_dsc`: seed `0x0143_1E28` twice with `--remove-member`; fingerprints match; `dsc=1`; production TCP ctor on the **removed** replica plants an uncommitted suffix then `discard_uncommitted_from`; RAM and disk suffix gone and `!is_member`. 0132 recover truncate is **not** this tooth. 0142 reader locality is **not** this tooth. Exit via `l28_tcp_dsc_ok`. AS-IS would skip discard (`ids` filter). Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `discard_uncommitted` `entry: discard_node_counts`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `discard_node_counts_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `discard_node_counts_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0142 P1.2 TCP reader-local gate is **done**; `finish_queued_propose` no-leader `ids.first()` persist-leader remains later; `residuals.json` `R-joint` owner 0143.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
  - `finish_queued_propose` no-leader still passes `ids.first()` as the persist-leader arg (later).
