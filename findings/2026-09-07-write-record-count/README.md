# WriteBatch count must equal decoded op count (no silent prefix)

**Date:** 2026-09-07
**Primary sources:**
- facebook/rocksdb `db/write_batch.cc` `WriteBatchInternal::Iterate`
  (excerpt `write_batch_iterate_count.txt`): if the handler consumed the
  whole batch and `found != WriteBatchInternal::Count(wb)`, return
  `Status::Corruption("WriteBatch has wrong count")`.
- Practitioner occurrence: apache/kvrocks#2766 (2025-02-03),
  `memtable, bg_error: Corruption: WriteBatch has wrong count`
  (`kvrocks-2766.md`).

## What the source actually says

The encoded count is part of the batch header. Decode that applies fewer
ops than the header claims is corruption, not a successful prefix.

## Used this turn

Catalog pair `write_record_count` (`data_fate`, entry
`write_record_count_ok`): Verus `lemma_prefix_is_not_ok` on production
`crates/pedradb-core/src/batch.rs`. rustc body stays last-wins (`usize`).
AS-IS is constantly true (silent prefix).

## Not claimed

Dump of `WriteRecord::decode`. “somos seL4”.
