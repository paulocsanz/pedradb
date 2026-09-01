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
  status: `todo`
- [ ] **P0.5** Measure: local 6M A/B, then guest run #20 at 25M — hydrate,
  settle, disk peak, read legs vs RocksDB default; verdict recorded in the
  findings README. — status: `todo`

### P1 — next wave (depends on P0 or clearly deferrable)

- [ ] **P1.1** Encode-path per-byte cut: bulk builder fills blocks straight
  from batch payload bytes (no per-entry InternalKey/Bytes allocations). —
  status: `part-done` (caller-side half shipped 2026-08-31: all four
  write-path callers re-opened every freshly written SST via `open_on`
  — a full read + per-block lz4 + per-entry decode pass; they now keep
  the writer's in-place table. Local 6M A/B: settle −26 %, hydrate −26 %
  median, paired, both rounds favoring the fix. Block-fill encode cut
  remains.)
- [ ] **P1.2** Batch MANIFEST persists across consecutive chunk installs. —
  status: `todo`
- [ ] **P1.3** Chunk-size sweep for read legs at 25M (64 vs 128 MiB) —
  run #19 showed fewer/bigger files improve probe/get legs. — status:
  `todo`

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
| P0.5 | p0 | Local A/B + guest verdict | in-progress | local 6M A/B done (`9698caf`): hydrate −39…−45 %, settle −25…−43 %, 22 BULKDIAG on-arm / 0 off; guest run #23 in flight | 2026-09-01 |
| P1.1 | p1 | Zero-alloc bulk encode | part (caller-side read-back removed) | `db.rs` `table.rs` | 2026-08-31 |
| P1.2 | p1 | Batched manifest persists | todo | — | 2026-08-31 |
| P1.3 | p1 | Chunk-size sweep for reads | todo | — | 2026-08-31 |
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
