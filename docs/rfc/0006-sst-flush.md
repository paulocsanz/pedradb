# RFC-0006: SST flush + reopen (P1.1)

**Status:** done  
**Updated:** 2026-08-11  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md) P1.1

---

## Solution

- On-disk SST v1 (`PEDRSST\0`): sorted `InternalKey` + value entries + max sequence.  
- `Db::flush()` → write `NNNNNN.sst`, fsync, clear MemTable, rotate `CURRENT.log`.  
- `Db::open` loads all `*.sst` (oldest→newest), then recovers WAL.  
- `get` / TX reads: MemTable first, then SSTs newest-first.

## Out of scope (later P1)

- Block/index/Bloom, compaction, public range merge, auto-flush threshold.
