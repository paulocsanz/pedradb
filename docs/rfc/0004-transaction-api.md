# RFC-0004: Multi-key Transaction API (P0.4)

**Status:** done  
**Updated:** 2026-08-11  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md) P0.4  
**Depends:** [RFC-0003](0003-wal-recover-basic-engine.md)

---

## Background

P0.3 has auto-commit `Db::put`/`delete`. The product sentence requires multi-key atomic updates (row + index) in one commit.

## Proposed solution

```text
let mut tx = db.begin();
tx.put(b"row", …);
tx.put(b"idx", …);
tx.commit()?;   // one WriteRecord, N sequences, sync once
```

- **Snapshot** at `begin` = `last_sequence()` (O5).  
- **Staging** buffer for puts/deletes; reads see staging then snapshot.  
- **Single-writer:** `begin` takes `&mut Db` (no concurrent write TX).  
- **Commit:** one WAL record with all ops; all-or-nothing on recovery.  
- **Abort / Drop:** discard staging.

## Delivery

- [x] P0.4a `Transaction` + begin/commit/abort — done (`tx.rs`)  
- [x] P0.4b multi-key atomicity + read-your-writes tests — done  
- [x] P0.4c reopen recovers multi-op commit — done  

## Status

| ID | Title | Status |
|----|-------|--------|
| P0.4a | Transaction API | done |
| P0.4b | Tests (RYW, abort, multi-key) | done |
| P0.4c | Reopen multi-op | done |

## Out of scope

OCC multi-writer, range in TX, nested TX.
