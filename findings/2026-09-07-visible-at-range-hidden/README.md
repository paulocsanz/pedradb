# A Value is live at a snapshot iff it is not range-hidden; deletions are not live

**Date:** 2026-09-07
**Primary sources:**
- RocksDB wiki *DeleteRange Implementation* (point lookups):
  `FragmentedRangeTombstoneIterator::MaxCoveringTombstoneSeqnum` — if a
  covering tombstone is found, the point value is deleted.
- maxnilz 2026-01-13 *Range Tombstones and Point Lookups*
  (https://maxnilz.com/posts/033-cckv-range-tombstone): “Is this key
  visible at the given version?” / covered by a range deletion at or
  before the snapshot.

## What the source actually says

A `kTypeValue` under a covering range tombstone is not live. A
`kTypeDeletion` is not a live value. Treating every version as live
(the as-is mutant) is the F30 scan leak.

## Used this turn

Catalog pair `visible_at` (`data_fate`, entry `visible_at`): Verus
`lemma_deletion_is_hidden` + `lemma_range_hidden_value_is_hidden` on
production `crates/pedradb-core/src/merge.rs`. rustc `key::ValueType`
body stays last-wins. Pair `range_covers` keeps its model twin.

## Not claimed

Fragmented range tombstone index. Dump of `StreamingVisibleIter`.
“somos seL4”.
