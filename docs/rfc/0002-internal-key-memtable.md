# RFC-0002: InternalKey + MemTable (P0.2)

**Status:** done  
**Updated:** 2026-08-11  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md)  
**Slice:** P0.2 of the high-level delivery plan

---

## Background

- PedraDB core ships a crash-safe **WAL** (P0.1) but has no in-memory store yet.
- Multi-key ACID needs a **versioned ordered map** before TX (P0.4) and recovery (P0.3).
- Peers (RocksDB, Pebble, LevelDB) use an **internal key** = user key + sequence + type.

### Why now

P0.2 is the smallest next vertical: pure in-memory, no disk format beyond optional encode helpers for later WAL payloads. Unblocks P0.3 (replay WAL → MemTable).

---

## Problems this solves

- **Problem:** No place to hold puts/deletes with MVCC sequence numbers.  
- **Problem:** Encoded-string internal keys (LevelDB style) waste allocations; Pebble teaches **struct keys**.  
- **Problem:** Need a stable key order for `get` at a snapshot and for future range scans.

---

## Proposed solution

1. **`InternalKey`** — `{ user_key: Bytes, sequence: u64, kind: ValueType }` with RocksDB-compatible order:
   - user key ascending (bytewise)
   - sequence **descending** (newest first)
   - kind descending
2. **`ValueType`** — `Deletion = 0`, `Value = 1` (extensible later).
3. **`MemTable`** — ordered map of internal keys → value bytes; `get`/`put`/`delete`/`range` at a snapshot sequence; approximate memory accounting.
4. **No arena yet** — `Bytes` ownership is enough for P0; fixed arena is a later optimization (docs may still mention it).
5. **No public `Db` yet** — this slice is internal engine surface (`pub` within core for tests and next slices).

---

## Delivery slices

### P0 — this RFC

- [x] **P0.1** (parent) Crash-safe WAL — status: `done`  
- [x] **P0.2a** `ValueType` + `SequenceNumber` + `InternalKey` (`Ord`, encode/decode) — status: `done`  
- [x] **P0.2b** `MemTable` insert / get-at-snapshot / delete / range — status: `done`  
- [x] **P0.2c** Unit tests (order, overwrite, tombstone, snapshot visibility) — status: `done`  

### P1 — next waves (other RFCs / parent P0.3+)

- [x] **P1.1** WAL record payload for put/delete + recover into MemTable — status: `done` (shipped in [RFC-0003](0003-wal-recover-basic-engine.md))  
- [x] **P1.2** Fixed-size arena / skip-list if benches demand — status: `done` (explicit **no**: BTreeMem stays; re-open only under [RFC-0012](0012-next-significant-steps.md) if benches require)  

### P2

- [x] **P2.1** Research opts for this module — status: `done` (none in charter; engine research → RFC-0012)  

---

## Status (living)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.2a | p0 | InternalKey + encode | done | `key.rs` | 2026-08-11 |
| P0.2b | p0 | MemTable API | done | `memtable.rs` | 2026-08-11 |
| P0.2c | p0 | Tests | done | 14 unit tests | 2026-08-11 |
| P1.1 | p1 | WAL ↔ MemTable recover | done | RFC-0003 | 2026-08-11 |
| P1.2 | p2 | Arena / skiplist | done | deferred → 0012 if needed | 2026-08-11 |
| P2.1 | p2 | Module research | done | none / → 0012 | 2026-08-11 |

---

## Acceptance criteria

### Tests

- InternalKey ordering: same user key, higher sequence sorts first.  
- Encode/decode round-trip.  
- MemTable: put then get; put overwrite (newer seq wins at high snapshot); delete hides value; old snapshot still sees pre-delete value.  
- Range yields user keys in order at a snapshot (visible versions only).  

### Telemetry / analytics

- None — library slice.

### Documentation

- This RFC + module-level rustdoc.  
- Parent RFC-0001 status row P0.2 → `done` when all P0.2* items land.  

### Screenshots

- backend-only.

---

## Out of scope

- Public `Db` / `Transaction` API  
- WAL recovery integration (P0.3)  
- SST, flush, compaction  
- Concurrent writers inside MemTable (single-threaded mutation for P0; outer lock later)  
- Fixed arena, skiplist  

---

## Locked for this slice

| ID | Choice |
|----|--------|
| K1 | Struct `InternalKey`, not encoded string on hot path |
| K2 | Sequence 56-bit usable range (RocksDB-compatible packing) |
| K3 | `BTreeMap` backend for correctness first |
| K4 | Snapshot = “see entries with `sequence <= snapshot`” |
| K5 | Tombstone = `ValueType::Deletion` hides older values for newer snapshots |
