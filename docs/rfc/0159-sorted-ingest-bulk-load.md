# RFC-0159: Sorted-ingest fast path (bulk load for append-only hydrate)

**Status:** in-progress
**Updated:** 2026-08-31

## Background

- Slipstream PR 19's hydrate feeds 25M entries as 24,414 apply batches of
  1024 puts. The bench's key function is fixed-width
  (`route.svc-{:06}.{:08}`) and the feed loop is strictly ascending —
  **every batch is sorted and sorts before the next** (verified in the
  vendored bench source, `benches/snapshot_backends.rs::hydrate`).
- We push that pre-sorted stream through the full LSM ladder: BTree
  memtable, WAL copy, 26 L0 flushes, L0→L1 merges, L1→L2/L2→L3 pushdowns,
  settle drain — ~25–30 GiB of logical bytes through the encode/decode
  loop for a 5.15 GiB result.
- Measured on the CHV guest (25M): the loop runs at ~110 MiB/s per job
  (CPU-bound, one effective core; disk does 381 MiB/s single-stream), so
  hydrate is 143.7–161.2 s and settle 47.9–88.2 s vs RocksDB default's
  25.3 s + 8.3 s. Removing CPU overheads (manifest storm, SST read-back —
  v21p) did not move the totals; wall time is **volume × per-byte rate**
  (runs #15/#16/#18/#19, `findings/2026-08-30-slipstream-compat-perf/`).
- RocksDB processes the same stream at 207 MiB/s logical **through its
  whole ladder** on the same core — the per-byte bar for our writer.

Evidence base: `findings/2026-08-30-slipstream-compat-perf/
sorted-ingest-architecture.md` (+ `run19-v21p-guest-25m.txt`).

## Problems This Solves

- **Problem:** hydrate/settle at 25M run at 0.17×/0.09× of RocksDB default
  because the ladder rewrites an append-only stream 4–5×.
- **Problem:** settle's wall time is unbounded by anything structural —
  it is just the leftover ladder volume (23 sequential pushdowns, 77 s in
  run #19).
- **Problem:** the 100M scale rung peaks at ~47 GiB on disk during the
  ladder for a ~20 GiB live set — the ladder, not the data, sets the
  peak.

## Proposed Solution

- Detect ascending, above-high-water apply batches per column family in
  the core write path; after consecutive confirmations, latch the CF into
  **append mode**. Any out-of-order/duplicate/delete-in-stream batch
  unlatches and routes through the normal path (conservative fallback; no
  behavior change for random workloads).
- In append mode, entries accumulate in a sorted-run builder (no BTree) and
  64–128 MiB chunks are written directly and **installed as disjoint
  bottom-level runs** — the leveled structure's own invariant, reached in
  one step instead of a pushdown ladder.
- The WAL covers only the uninstalled tail; installed chunks' segments are
  GC-able after the manifest persist (WAL steady state ≈ one chunk).
- `settle()` on an append-mode DB flushes the tail; the drain finds clean
  levels and no-ops.
- Alongside (P1): cut per-entry encode cost on the bulk path (zero-alloc
  block fill from the batch payload) — the volume cut alone tops out at
  ~0.5×; ≥1× vs rocks needs both.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)

- [x] **P0.1** Sorted-stream detector: per-CF ascending + above-high-water
  decision on the batch ops, latch state machine, conservative fallback
  semantics — pure logic + unit/kernel tests. — status: `done`
  (`crates/pedradb-core/src/bulk_ingest.rs`, 11 tests)
- [x] **P0.2** Flush-path direct-to-bottom install: at flush time, a latched
  family's pure-append span (strictly-ascending puts, no tombstones, hull
  disjoint from the family's files at levels ≥ 1) installs its SST at
  `MAX_LSM_LEVEL` instead of L0 — written once, never re-laddered, so settle
  has nothing to push down. Ladder fallback on every disqualifier.
  `PEDRA_BULK=0` kill switch; `PEDRA_BULK_DIAG` prints each non-L0 install
  (`db.rs: bulk_span_level`). — status: `done` (4 tests in `db::tests`;
  2026-09-01 follow-up: the `ConcurrentDb` install sites — `flush`,
  `drain_imm_once`, `materialize_parked_once` — were unwired, so run #22
  benched P0.2 inert (BULKDIAG=0, settle unchanged). Write-side observation
  was already live (`commit_async_ops` / `commit_async_one`); the fix routes
  those three installs through `bulk_span_level` — 3 more tests in
  `concurrent::tests`.)
70→- [ ] **P0.3** WAL ring for append mode: uninstalled tail only, segment GC
  after install, crash-replay equality test. — status: `todo`
- [ ] **P0.4** End-to-end regression set: sorted ingest then
  gets/scans/probes equal the ladder path byte-for-byte; out-of-order
  mid-stream falls back and stays correct; settle no-ops on clean levels. —
  status: `part-done` (core half shipped 2026-09-01:
  `concurrent::tests::bulk_twin_matches_ladder_after_settle` — bulk vs
  ladder twin over identical batches incl. a mid-stream descent,
  equal after settle; compat/E2E half pending)
- [x] **P0.5** Measure: local 6M A/B, then guest run at 25M — hydrate,
  settle, disk peak, read legs vs RocksDB default; verdict recorded in the
  findings README. — status: `done` (local A/B hydrate −39…−45 % /
  settle −25…−43 %, 22 vs 0 BULKDIAG; guest run #23: settle 2.3 s vs
  Rocks 8.3 s = 3.61×, hydrate 73.6 s = 0.34×, reads flat, 73 BULKDIAG)

### P1 — next wave (depends on P0 or clearly deferrable)

- [ ] **P1.1** Encode-path per-byte cut — re-aimed 2026-09-01: the guest's
  hydrate wall is SST **materialize** (run #23: FLUSHDUR sum 63.4 s of the
  73.6 s hydrate, 73 chunks × 867 ms ≈ 89 MiB/s), not commit encode (local
  phase profile: prepare+mem+real WAL encode ≈ 2.9 s per 6M; the apparent
  local WAL dominance is APFS `F_PREALLOCATE` — `findings/
  2026-08-30-slipstream-compat-perf/hydrate-phase-profile-6m.md`). Cuts in
  `write_sst_try_sorted_body`: entries encode straight into `block_buf`
  (was: scratch Vec + copy — one full extra pass per byte), `prev_ikey` kept
  by move, largest key derived after the loop (was: per-entry clone), and a
  first-block lz4 probe — truly incompressible payloads write v3 raw for the
  whole file, skipping lz4 CPU (~30 % of materialize) for ~10 % more disk;
  `PEDRA_LZ4_PROBE=0` kill switch; `PEDRA_FLUSH_STAGES` prints the per-stage
  split. Local verdict 2026-09-01: paired 6M A/B FLUSHDUR −13 %, settle
  1.1→0.4 s; the bench payload compresses 2.6× so the probe never trips
  (probe on/off disk totals identical to 113 B / 1.24 GiB) — probe ships as
  protection for incompressible data, lz4 stays on for the bench. — status:
  `in-progress` (code + tests + local A/B landed; guest run pending)
- [ ] **P1.2** Batch MANIFEST persists across consecutive chunk installs. —
  status: `todo`
- [ ] **P1.3** Chunk-size sweep for read legs at 25M (64 vs 128 MiB) —
  run #19 showed fewer/bigger files improve probe/get legs. Root cause
  found 2026-09-01: chunks staged at the GLOBAL auto-flush cap (compat
  DB-level default 64 MiB), not the per-CF buffer — `try_stage_if_full`
  used `auto_flush_threshold()` which ignored per-CF overrides; fix makes
  it max(global, per-CF) (`findings/
  2026-08-30-slipstream-compat-perf/p13-chunk-threshold-root-cause.md`);
  probe A/B 4→1 parks at the 16 MiB CF limit. Guest run #27 (v25, 25M):
  23×256 MiB chunks as designed but hydrate 116.0 s (+54% vs v24 75.6) —
  two mechanisms, each with its own counter: (a) `flush_check_ms` 18.7 →
  21474 — `MemTable::take_family` partitions by reinserting every key
  (256 MiB ≈ 2.6 M keys ≈ 0.9 s/chunk on the writer's commit path; never
  hit pre-fix because data never reached its per-CF limit in the active
  mem), fixed by a `split_off` node-move partition + subtractive stats
  (v27); (b) flush-debt at cap = one chunk parks the writer into
  2 ms-poll sleeps while the worker materializes (≈ 33 s dead wall,
  run #27b repeat reproduced 120.5 s), fixed by writer assist-drain:
  at debt ≥ cap the submit materializes one parked table inline (v26).
  Reads did NOT move with 23 vs 88 files — the v24/v25 read deltas were
  host-load contamination (gate load 34, six qemu at ~200%). — status:
  `v26+v27 landed, guest verification run pending (host still loaded)`

### P2 — later / polish

- [ ] **P2.1** Nearly-sorted tolerance (bounded out-of-order window) for
  real `watch_applied` feeds. — status: `todo`
- [ ] **P2.2** 100M scale rung via bulk mode (disk peak ≈ live set + one
  chunk). — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Sorted-stream detector + latch | done | `bulk_ingest.rs` | 2026-08-31 |
| P0.2 | p0 | Bulk flush-path install at bottom level | done | `db.rs` (`bulk_span_level`, 4 tests) + `concurrent.rs` funnels (3 tests) | 2026-09-01 |
| P0.3 | p0 | WAL ring for append mode | todo | — | 2026-08-31 |
| P0.4 | p0 | E2E regression set | todo | — | 2026-08-31 |
| P0.5 | p0 | Local A/B + guest verdict | done | local 6M A/B (`9698caf`): hydrate −39…−45 %, settle −25…−43 %; guest run #23 (25M): settle 84.9→2.3 s = **3.61× vs Rocks 8.3 s**, hydrate 157.0→73.6 s (0.34×), reads flat; 73 BULKDIAG (72 parked + 1 flush) | 2026-09-01 |
| P1.1 | p1 | Materialize per-byte cut (direct block encode + lz4 probe) | in-progress (code+tests+local A/B: FLUSHDUR −13 %, disk identical; guest pending) | `table.rs` | 2026-09-01 |
| P1.2 | p1 | Batched manifest persists | todo | — | 2026-08-31 |
| P1.3 | p1 | Chunk-size: per-CF buffer governs stage threshold | fix landed; guest run #27: 256MiB chunks regress hydrate +54% — (a) `take_family` reinsert loop 21.5s flush_check (fixed: split_off partition, v27) + (b) flush-debt sleep ping-pong ≈33s (fixed: writer assist-drain, v26); reads unaffected by chunk count (host-load contamination found); v26+v27 guest run pending | `concurrent.rs`, `memtable.rs`, `db.rs` | 2026-09-01 |
| P2.1 | p2 | Nearly-sorted window | todo | — | 2026-08-31 |
| P2.2 | p2 | 100M rung via bulk mode | todo | — | 2026-08-31 |

## Acceptance Criteria

- **Tests**
  - `bulk_latch_engages_on_ascending_batches` / unlatches on out-of-order,
    duplicate, delete-in-stream (P0.1)
  - `bulk_install_is_disjoint_bottom_run` + reads through the open tail
    (P0.2)
  - `bulk_crash_replay_equals_ladder_path` (kill after N installs; reopen;
    keyspace identical to a ladder-ingested twin) (P0.3)
  - `bulk_settle_noops_on_clean_levels`; `bulk_fallback_midstream_correct`
    (P0.4)
  - Core suite green (known pre-existing flakes excepted).
- **Telemetry / Analytics:** `PEDRA_BULK_DIAG=1` one line per chunk
  (cf, entries, logical bytes, chunk bytes, install ms) + latch/fallback
  counters on the existing flush diag cadence.
- **Documentation:** findings README run entry; this RFC's status table
  updated in the same commits as the code.
- **Screenshots:** none — backend-only.

## Out of scope

- Read-leg per-get cost (get_hit/prefix_scan at 25M) — separate track.
- Any compat (`rocksdb-compat`) API change; any raft/TX path change.
- SST format changes (no v6); compression changes.
- RocksDB parity claims outside the registered product decision (G1:
  fdatasync-before-Ok stands; slipstream hydrate applies `sync=false` on
  both peers).
