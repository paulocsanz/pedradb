# RFC-0003: WAL records + recover + basic engine (P0.3)

**Status:** done  
**Updated:** 2026-08-11  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md) P0.3  
**Depends:** [RFC-0002](0002-internal-key-memtable.md) (done)

---

## Background

P0.1 ships a physical WAL (blocks, CRC, fragments). P0.2 ships versioned MemTable.  
Nothing yet encodes puts/deletes into the WAL or rebuilds the MemTable on reopen.

## Problems this solves

- Process kill loses in-memory state without replay.  
- No minimal `Db` surface for put/get/delete before full `Transaction` (P0.4).

## Proposed solution

1. **Logical WAL payload v1** (self-describing): version, op kind, sequence, key, value.  
2. **`Db`**: open directory → recover WAL → MemTable; `put`/`delete`/`get` with per-op sequence; optional sync (default on).  
3. **Not yet:** interactive multi-key TX (P0.4), SST, concurrent writers.

## Delivery slices

### P0

- [x] **P0.3a** `batch` / write-record encode+decode — status: `done`  
- [x] **P0.3b** `Db::open` recover + put/delete/get + sync — status: `done`  
- [x] **P0.3c** Reopen tests (write, close, reopen) — status: `done`  

### P1 / later (parent RFC)

- Multi-op atomic batch in one WAL record for TX commit  
- WAL seek by offset (P1.6)

## Status

| ID | Title | Status | Updated |
|----|-------|--------|---------|
| P0.3a | Write record codec | done | 2026-08-11 |
| P0.3b | Db open/put/get/delete | done | 2026-08-11 |
| P0.3c | Reopen tests | done | 2026-08-11 |

## Acceptance

- Put then get; delete hides; reopen after close restores keys.  
- Truncated trailing WAL record does not corrupt recovery (inherits WAL reader).  
- Default write path fsyncs before returning Ok (O1).

## Out of scope

- Public `Transaction`  
- Multi-process open  
- SST / compaction  
