# RFC-0009: RocksDB-class engine maturity

**Status:** done (P0–P2 complete; maturity wave 2 → [RFC-0014](0014-rocks-pebble-redwood-maturity.md))  
**Updated:** 2026-08-12  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md)  
**Parallel tracks:** A (justify use) + B (engine depth)  
**Sibling:** [RFC-0010](0010-dbs-on-top.md) — products *on* PedraDB  

---

## Background

PedraDB P0–P2 delivered a **local** ordered KV with multi-key TX, WAL, SST flush, simple compact, `apply_batch`, and basic sim/oracle. That proves the product sentence, not production parity with RocksDB/Pebble under TiKV/Cockroach.

Today (facts):

- Sync-by-default commit ≈ one fsync per put/TX → low write thruput (mitigated by `WriteOptions::no_sync` + `Db::sync`).  
- MemTable is `BTreeMap`; SST v2 has block format + sparse index (entries still fully loaded for range/compact).  
- Auto-flush by MemTable size; auto-compact by SST count; compact/flush write `.sst.tmp` → rename; version GC via `CompactOptions` / `CompactGcOptions`.  
- `CURRENT` + `MANIFEST-*` inventory; cross-process PID `LOCK`; sim flush/compact faults + nth-op sweep.  
- Research opts (Bloom/value-log/LL) **explicitly deferred**.  
- TX multi-key remains a **strength** RocksDB lacks.

## Problems this solves

- **Problem:** Outer DBs cannot treat PedraDB as a drop-in RocksDB role without write amp control, GC, and durable checkpoint story.  
- **Problem:** Even embed users hit fsync-per-write and unbounded mem/WAL growth without auto-flush.  
- **Problem:** Research opts (WiscKey/Bloom/Lazy Leveling) were deferred (P2.2); need a measured path, not cargo-cult.

## Proposed solution

Three waves inside **this** crate (`pedradb-core`):

1. **A — Justify use / write path:** per-write sync options, group-style batching (many mutations → one fsync), auto-flush by size, tiny embed example (secondary index).  
2. **B1 — Real store:** block-oriented SST + sparse index, size-triggered compact policy, version GC horizon.  
3. **B2 — Ops & trust:** MANIFEST/CURRENT, crash-safe compact, file lock, richer sim, optional measured research opts.

Public API stays small: `open / begin / get / put / delete / range / commit / apply_batch / flush / compact / snapshot` plus knobs that default safely.

## Delivery slices

### P0 — write path + auto-flush (Track A)

- [x] **P0.1** `WriteOptions { sync }` on put/delete/apply_batch/commit path — status: `done`  
- [x] **P0.2** Auto-flush when MemTable approx size ≥ threshold — status: `done`  
- [x] **P0.3** Embed example: secondary-index layer on multi-key TX — status: `done`  
- [x] **P0.4** Bench note: sync-every vs sync-batched thruput — status: `done` (`benches/baseline.rs`)  

### P1 — LSM depth (Track B1)

- [x] **P1.1** SST block format + sparse index (v2 on-disk; entries still materialized for range/compact) — status: `done`  
- [x] **P1.2** Compaction policy by count (`OpenOptions::auto_compact_sst_count`; size-based deferred) — status: `done`  
- [x] **P1.3** Version GC (`CompactGcOptions::{min_sequence,keep_only_latest}` via `compact_with`) — status: `done`  
- [x] **P1.4** Crash-safe compact (`.sst.tmp` + rename; orphan cleanup on open) — status: `done`  

### P2 — ops polish (Track B2)

- [x] **P2.1** MANIFEST / file inventory + recovery of mid-compact — status: `done` (`CURRENT` + `MANIFEST-*`, orphan SST GC)  
- [x] **P2.2** Exclusive dir lock (cross-process PID `LOCK`; same-PID re-open steals for crash-sim) — status: `done`  
- [x] **P2.3** Measured research opt **or** keep deferred (Bloom/value-log/LL) — status: `done` (**deferred** until thruput baselines warrant)  
- [x] **P2.4** Sim: inject faults around flush/compact — status: `done` ([RFC-0011](0011-env-fault-injection.md) P1 + sim compact/nth sweep)  

## Status (living)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | WriteOptions sync | done | WriteOptions / put_with | 2026-08-11 |
| P0.2 | p0 | Auto-flush threshold | done | OpenOptions.auto_flush_bytes | 2026-08-11 |
| P0.3 | p0 | Index-layer example | done | examples/ (gallery: hello → bank) | 2026-09-04 |
| P0.4 | p0 | Bench sync vs batched | done | put_sync_each vs put_nosync_then_sync | 2026-08-11 |
| P1.1 | p1 | Block SST + index | done | SST v2 BLOCK_TARGET + index | 2026-08-11 |
| P1.2 | p1 | Count-based auto-compact | done | OpenOptions.auto_compact_sst_count | 2026-08-11 |
| P1.3 | p1 | Version GC | done | CompactOptions + gc_compact_entries | 2026-08-11 |
| P1.4 | p1 | Crash-safe compact | done | .sst.tmp rename + cleanup_tmp_ssts | 2026-08-11 |
| P2.1 | p2 | MANIFEST | done | CURRENT + MANIFEST-* + orphan GC | 2026-08-11 |
| P2.2 | p2 | Dir lock | done | LOCK PID / OpenOptions.exclusive | 2026-08-11 |
| P2.3 | p2 | Research opts measured | done | deferred (explicit) | 2026-08-11 |
| P2.4 | p2 | Sim flush/compact faults | done | RFC-0011 + sim compact/nth sweep | 2026-08-11 |

## Acceptance criteria

### Tests
- Put with `sync: false` then process crash may lose tail; after explicit `sync` or auto-group, durable.  
- Auto-flush: after N large puts, `sst_count() >= 1` without manual `flush`.  
- Example index layer: row+idx commit atomic; abort leaves neither.  
- (P1+) Get after block-SST reopen; compact crash recovery; GC drops old versions.

### Telemetry / analytics
- None required; optional counters later (flush count, fsync count).

### Documentation
- This RFC + `docs/usage.md` updates for WriteOptions / auto-flush.  
- Cross-link RFC-0010 for “when engine is enough for outer DB”.

### Screenshots
- backend-only.

## Out of scope

- Multi-node Raft / SQL / etcd (RFC-0010).  
- Matching RocksDB feature surface or on-disk compatibility.  
- OCC multi-writer (still single-writer unless RFC-0001 O2 reopened).

## Parallelism with RFC-0010

| Track | Owner RFC | Can start now? |
|-------|-----------|----------------|
| A write path / auto-flush / example | **0009 P0** | **Yes** — no outer product needed |
| B block SST / GC / MANIFEST | **0009 P1–P2** | Yes, independent of Raft |
| C apply loop / Raft / SQL | **0010** | Yes against **current** `apply_batch` + snapshot; hardens as 0009 lands |
