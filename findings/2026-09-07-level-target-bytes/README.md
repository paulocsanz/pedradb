# Level target bytes: exponential ladder, never wrap

**Date:** 2026-09-07
**Primary source:** RocksDB wiki *Leveled Compaction*, mirror
`EighteenZi/rocksdb_wiki` `Leveled-Compaction.md`. Raw: that file.

## What the source actually says

When `level_compaction_dynamic_level_bytes` is false, L1's target is
`max_bytes_for_level_base` and
`Target_Size(Ln+1) = Target_Size(Ln) * max_bytes_for_level_multiplier`
(default multiplier 10). Targets grow with the level. They do not wrap
downward.

## Used this turn

Catalog pair `leveling` (`data_fate`, entry `level_target_bytes`):
`l1_target * 10^min(level-1, 18)` saturating, never wrapping. AS-IS uses
wrapping_pow so a deep level reads under target. Production `leveling.rs`
is now the Verus term (`single_artifact`). Lemma `lemma_targets_monotone`
is the second possibility. Pairs `leveling_pick` / `leveling_pushdown`
keep twins until their turns.

## Not claimed

RocksDB dynamic_level_bytes. Dump of `prepare_l0_compact`. “somos seL4”.
