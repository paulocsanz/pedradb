# Pushdown picks the oldest source file plus GetOverlappingInputs, and refuses a stacked destination

**Date:** 2026-09-07
**Primary sources:**
- RocksDB wiki *Leveled Compaction* (EighteenZi mirror, already persisted
  under `findings/2026-09-07-level-target-bytes/Leveled-Compaction.md`):
  after L0→L1, “pick at least one file from L1 and merge it with the
  overlapping range of L2”. Non-zero levels are mutually non-overlapping.
- facebook/rocksdb `LevelCompactionBuilder::PickFileToCompact` in
  `findings/2026-09-07-leveling-pick-overlap-slice/compaction_picker_level.cc`:
  L1+ starts with one file; output-level inputs are
  `GetOverlappingInputs` of that file’s range.

## What the source actually says

A pushdown job is one source file plus the overlapping destination
slice. L1+ is assumed disjoint. Pedra’s extra gate (`is_disjoint` /
`disjoint_spec`) refuses the job when the destination is still stacked
(the pre-leveled shape); the as-is mutant (`pick_pushdown_as_is_blind`)
rewrites stacked levels anyway.

## Used this turn

Catalog pair `leveling_pushdown` (`data_fate`, entry `pick_pushdown`):
production `leveling.rs` is already the Verus term (ported with
`leveling_pick`). This turn flags `single_artifact`. Plant
`pick_pushdown_on_live_pushdown_gate_is_not_ok`. Divergence
`divergence_pushdown_blind` is the second possibility.

## Not claimed

RocksDB round-robin multi-file expansion. Dump of
`prepare_pushdown_compact`. “somos seL4”.
