# RFC-0014: Toward RocksDB / Pebble / Redwood engine maturity

**Status:** done (P0–P2)  
**Updated:** 2026-08-12  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md)  
**Predecessor:** [RFC-0009](0009-rocksdb-class-engine.md) (kernel depth wave 1 — **done**)  
**Research reopen:** supersedes non-ship of Bloom in [0012-research-decisions](0012-research-decisions.md) for filter-only  

---

## Background

RFC-0009 delivered a **usable** local LSM: WAL, TX, SST v2 blocks, MANIFEST, auto-flush, fault seams. That is **not** yet RocksDB/Pebble/Redwood-class as an infrastructure primitive.

Honest gap (2026-08-12):

| Axis | Rocks / Pebble / Redwood | PedraDB after 0009 | This RFC |
|------|--------------------------|--------------------|----------|
| Durability contract | Documented + field lore | Explicit fsync + hunt F1–F21 | Keep; expand verify/ops |
| Point lookup amp | Bloom + cache + levels | Full scan of open tables | **Bloom + key bounds** |
| Range | Streaming iterators + limit | Materialized `Vec` of all layers | **Bounded range + limit** |
| Ops | Checkpoint, properties, checksums | Manual dir copy | **Checkpoint / stats / verify** |
| Compaction | Leveled / size-tiered | Whole-merge + count trigger | Count **or SST bytes** trigger |
| Multi-writer / TB scale | Yes | Single-writer; mem-bound | **Out of P0** (P2+) |
| Simulation / field time | Years | Lab DST | Continuous hunt (external) |

**Doctrine:** close *utility and integrity surfaces* that outer products (Montanha, DCS) need before chasing multi-TB write amp papers.

**Ceiling / option preservation / sled-shaped layer:**  
[`../performance-ceiling-option-preservation-and-sled-layer.md`](../performance-ceiling-option-preservation-and-sled-layer.md)
(K1–K3 phases align with this RFC’s P1–P2; do not ship dual B-tree store).

---

## Problems this solves

- **Problem:** `range` cloned every internal version in every SST → OOM risk on large DBs.  
- **Problem:** Point get always walked tables with no negative filter (Rocks/Pebble bloom).  
- **Problem:** No first-class backup point (RocksDB Checkpoint).  
- **Problem:** No `GetProperty`-class stats or checksum verification API.  
- **Problem:** Research doc froze Bloom as non-ship without a measured reopen path that products need for correct *shape*, not only thruput.

---

## Proposed solution

### Shipped in P0 (this change set)

1. **SST v3** — block layout + **on-disk Bloom filter**; v1/v2 still readable; v2 rebuilds bloom in memory on open.  
2. **Key bounds** on each `SstTable` (`smallest`/`largest` user key) for get/range prune.  
3. **`range_limited` / `range_at_limited`** + `visible_range_limited` — page without unbounded result sets; SST `entries_in_user_range` avoids full-table clone.  
4. **`Db::create_checkpoint`** — flush, copy CURRENT/MANIFEST/SSTs/WAL + `CHECKPOINT` meta (openable as a normal DB dir).  
5. **`Db::stats` → `DbStats`** — sequences, mem/SST counts and bytes, WAL size.  
6. **`Db::verify_checksums`** — re-open live SSTs + WAL recover (CRC fail-stop).  
7. **`OpenOptions::auto_compact_sst_bytes`** — size-triggered whole-merge compact.  
8. **`Env::copy_file`** default — checkpoint through the Env seam (fault-injectable).

### P1 (shipped)

- True **streaming** range iterator (`StreamingVisibleIter` / `Db::scan`) — no full-keyspace `Vec` before yield.  
- Lazy **block load** for v2+ SST: retain compressed payload + index; expand one block for get; bounded range loads overlapping blocks only; full materialize only for compact.  
- **Leveled** multi-level compact (`sst_levels`, L0→L1+ subset).  
- **lz4** compression (SST v4 writer default).  
- Table cache + block cache.

### P2 (shipped)

- **OCC multi-writer:** [`ConcurrentDb::begin_occ`] / [`OccTransaction`] — snapshot + read/write-set validation; `TransactionConflict` fail-closed (not coarse-mutex-only “multi-writer”).  
- **Value log (WiscKey-shaped):** `OpenOptions::large_value_threshold` spills large puts to `VALUES.vlog`; SST/mem store `VLG1` pointers; **GC deferred** (file grows).  
- **Incremental backup:** `BackupEngine::create_incremental` (= `ship_wal`), `list_increments`, `restore_with_increments` + existing PITR.  
- Match Pebble range-delete / ingestion semantics only if Montanha needs them (still optional later).

---

## Delivery slices

### P0 — integrity + ops surfaces

- [x] **P0.1** SST v3 + Bloom filter — status: `done` (`bloom.rs`, `sst/table.rs`)  
- [x] **P0.2** Range prune + `range_limited` — status: `done`  
- [x] **P0.3** Checkpoint API — status: `done` (`Db::create_checkpoint`)  
- [x] **P0.4** Stats + verify_checksums — status: `done`  
- [x] **P0.5** Size-based auto-compact knob — status: `done`  

### P1 — memory / amp

- [x] **P1.1** Streaming range iterator — status: `done` (`StreamingVisibleIter`, `Db::scan` / `scan_at`)  
- [x] **P1.2** Lazy SST block load — status: `done` (v2+ payload retained; `decode_block` / `point_at`)  
- [x] **P1.3** Multi-level or size-tiered compact — status: `done` (leveled L0→L1+)  
- [x] **P1.4** Optional compression — status: `done` (SST v4 lz4 default writer)  

### P2 — scale class

- [x] **P2.1** Multi-writer OCC — status: `done` (`occ.rs`, `ConcurrentDb::begin_occ`)  
- [x] **P2.2** Value log (threshold spill; GC deferred) — status: `done` (`vlog.rs`, `large_value_threshold`)  
- [x] **P2.3** Incremental backup / restore tooling — status: `done` (`create_incremental`, `restore_with_increments`)  

---

## Status (living)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | SST v3 + Bloom | done | core bloom + write path | 2026-08-12 |
| P0.2 | p0 | range_limited + prune | done | merge + SstTable + Db | 2026-08-12 |
| P0.3 | p0 | Checkpoint | done | create_checkpoint | 2026-08-12 |
| P0.4 | p0 | Stats + verify | done | DbStats / verify_checksums | 2026-08-12 |
| P0.5 | p0 | auto_compact_sst_bytes | done | OpenOptions | 2026-08-12 |
| P1.1 | p1 | Streaming range | done | merge + Db::scan | 2026-08-12 |
| P1.2 | p1 | Lazy block load | done | SstTable payload + decode_block | 2026-08-12 |
| P1.3 | p1 | Leveled/size-tier compact | done | compact_levels + MANIFEST levels | 2026-08-12 |
| P1.4 | p1 | Compression | done | SST v4 lz4 | 2026-08-12 |
| P2.1 | p2 | Multi-writer OCC | done | OccTransaction + conflict tests | 2026-08-12 |
| P2.2 | p2 | Value log | done | threshold spill; GC deferred | 2026-08-12 |
| P2.3 | p2 | Incremental backup | done | create_incremental + restore_with_increments | 2026-08-12 |

---

## Acceptance criteria

### Tests
- [x] SST v3 round-trip has active bloom; absent keys do not return false values.  
- [x] `range_limited` returns at most N live keys after flush.  
- [x] Checkpoint directory opens and preserves pre-checkpoint keys only.  
- [x] `stats()` reports non-zero `sst_bytes` after flush; `verify_checksums` Ok on healthy DB.  
- [x] `entries_in_user_range` returns only keys in bounds.  
- [x] OCC: two overlapping commits on same key → one `TransactionConflict`; concurrent threads Ok|Conflict only.  
- [x] Vlog: large put → reopen → get recovers payload; `VALUES.vlog` present.  
- [x] Incremental: two `create_incremental` ships → `restore_with_increments` sees all keys.

### Documentation
- This RFC + `docs/usage.md` API rows + research decision reopen note.  
- Honest: still **not** a drop-in Rocks/Pebble/Redwood for multi-TB multi-writer.

### Screenshots
- backend-only.

---

## Out of scope

- On-disk compatibility with RocksDB/Pebble file formats.  
- Claiming production parity or “more robust than RocksDB”.  
- Distributed store maturity (RFC-0013 Montanha) beyond what core exposes.

---

## Relationship to Redwood / FDB

FoundationDB’s storage robustness comes as much from **simulation** and transaction system as from Redwood’s B-tree. PedraDB’s path is:

1. Kernel integrity + ops (this RFC).  
2. Continuous DST (`pedradb-dst` / FailingEnv).  
3. Montanha multi-Raft product on top (RFC-0013).

Bloom/checkpoint/verify are **necessary but not sufficient** for FDB-class confidence.
