# RFC-0007: P1 range, compaction, WAL seek, benches

**Status:** done  
**Updated:** 2026-08-11  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md)

| Slice | Delivery |
|-------|----------|
| P1.2 | `Db::range` / `range_at` via `merge::visible_range` over MemTable ∪ SSTs |
| P1.3 | `Db::compact` — flush then merge all SSTs into one file |
| P1.4 | **N/A** — single-writer retained (O2); OCC deferred past P1 |
| P1.5 | `cargo bench -p pedradb-core --bench baseline` (put/get/commit) |
| P1.6 | `Wal::recover_from_offset` + `stream_position`; `WalReader::from_offset` |
