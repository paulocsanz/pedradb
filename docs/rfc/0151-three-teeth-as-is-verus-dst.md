# RFC: 0151 — Three teeth: AS-IS + Verus + named DST plant on every data_fate kernel

**Status:** done
**Updated:** 2026-08-29
**Parents:** [0150](0150-cf-family-visible-at-writerecord-pin-2pl.md), [0056](0056-one-hundred-percent-delivery.md), [0067](0067-dst-queued-rpc-pin-fail-closed.md), crash-dictionary

**Residual:** glue TCB (not `never_floor`). No extract of `db.rs`. L28 real TCP stays a campaign, not ∀ traces. CRC collision / Linux fsync / io_uring ring remain axioms.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or seL4-class. The sentence that *is* refused: a `data_fate` kernel whose AS-IS mutant would be silent-wrong has no named DST plant, or a Raft kernel planted only on Direct RPC.

## Background

- Catalog freeze already required a production `entry`, a Verus twin, and (for `data_fate`) named handlers.
- DST teeth existed as `_on_live_…_is_not_ok` tests and World/FailingEnv scenarios, but a new pair could land without a plant that **names** the kernel.
- RFC-0150 shipped CF / `visible_at` / WriteRecord / pin∘GC / 2PL kernels without wiring them into that freeze.
- Flush publish order (SST durable before MANIFEST) and the compat iterator window vs snapshot merge were production `if`s, not catalog pairs.

## Problems This Solves

- **Problem:** Verus and DST could refuse different sentences. A twin without a plant is a cartoon; a plant without a twin is a test.
- **Problem:** Raft plants could run on Direct RPC (the lab leftover RFC-0067 closed in production).
- **Problem:** MANIFEST could be described as published before the SST was durable; a windowed iterator could re-emit a version `visible_at` hid.

## Proposed Solution

- Freeze: every catalog `data_fate` pair records `as_is` (fn in the kernel file) and `dst_plant` `{file, test}`. `--lint` / `--ci` fail naming the pair id if either is missing, if the test does not call `entry(`, or if a Raft / `l28_*` plant does not mention `pin_dst_queued` or `RpcMode::Queued`.
- Register existing `_on_live_` / FailingEnv / World teeth. Do not invent 75 swarm seeds. Do not extract `db.rs`.
- Extract flush publish (`may_publish_manifest`) and iterator window (`iter_window_keep`) as production kernels the live paths already call.

## Delivery slices (mandatory)

### P0 — must ship first (freeze + 0150 / crash-dictionary plants)

- [x] **P0.1** `--lint` fails a `data_fate` pair missing AS-IS, twin, or named DST plant — status: `done`
- [x] **P0.2** Six RFC-0150 entries plus crash-dictionary teeth (`crash_after_sync`, CRC fail-stop, torn WAL) have plants that drive the shipped fn and assert the AS-IS dente — status: `done`

### P1 — next wave (put→Ok→crash→get surface)

- [x] **P1.1** Flush publish order (SST durable before MANIFEST) is a production kernel + AS-IS + twin + DST plant — status: `done`
- [x] **P1.2** Compat iterator window vs `visible_at` is a production kernel + AS-IS + twin + DST plant — status: `done`

### P2 — later (cluster)

- [x] **P2.1** Every Raft `data_fate` pair has a Queued-RPC plant — status: `done`
- [x] **P2.2** L28 real TCP stays campaign-not-theorem (`world_seed_l28_ok(0, false)` is not Ok) — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | three-teeth freeze | done | `check_three_teeth` / `test_three_teeth_freeze.py` | 2026-08-29 |
| P0.2 | p0 | 0150 + crash-dictionary plants | done | `*_on_live_*` + sim `crash_after_sync` / exploded CRC | 2026-08-29 |
| P1.1 | p1 | flush publish kernel | done | `may_publish_manifest` | 2026-08-29 |
| P1.2 | p1 | iterator window kernel | done | `iter_window_keep` | 2026-08-29 |
| P2.1 | p2 | Raft Queued plants | done | `three_teeth_queued.rs` | 2026-08-29 |
| P2.2 | p2 | L28 campaign not ∀ | done | `l28_real_tcp_is_campaign_not_forall` | 2026-08-29 |

## Acceptance Criteria

- **Tests**
  - `python3 scripts/formal/pedra_formal.py --lint` green; `python3 scripts/formal/test_three_teeth_freeze.py` drops `dst_plant` / `as_is` and names the pair id.
  - 0150 plants (FailingEnv, `crates/pedradb-sim/src/three_teeth_plants.rs`): CF scan leak, range-del scan, torn WriteRecord, pin∘`compact_reclaim`. 2PL: `LockTable::lock` cycle in `wait_for_deadlock_on_live_cycle_is_not_ok`.
  - Crash-dictionary: `crash_after_sync_recovers_committed` (dictionary_link), `recover_collect_act_on_live_exploded_crc_is_not_ok` (wal_recover), `reopen_outcome_on_live_crc_is_not_ok`.
  - P1: `persist_manifest` writes CURRENT only if `may_publish_manifest(sst_durable)` (`sst_durable = fsync_ok ∧ unsynced empty`). FailingEnv SST-sync-fail plant: L0 is in memory (`unsynced_sst_count > 0`), CURRENT unchanged, crash-reopen from WAL. AS-IS would publish anyway. Iterator: `try_scan_window_at` yields `snapshot_live`; `page_forward`/`page_last_n` call catalog `iter_window_keep(row.snapshot_live)`. `glue.db_rs_extracted` stays false.
  - P2: Raft plants in `three_teeth_queued.rs` mention `pin_dst_queued` / `RpcMode::Queued`; `world_seed_l28_ok(0, false)` is not Ok.
- **Telemetry / Analytics:** none — safety freeze.
- **Documentation:** this RFC; coverage-map rows; catalog `as_is` / `dst_plant`; residuals glue recount.
- **Screenshots:** backend-only.

## Out of scope

- Extracting `db.rs`. New World swarm seeds. Proving Linux `fsync` / CRC collision / io_uring ring / `∀` ConcurrentDb interleavings.
- Making L28 real TCP a theorem.
- More CRC-mismatch twins or membership-locality RFCs.
- crates.io, Lean/Aeneas second machine.
