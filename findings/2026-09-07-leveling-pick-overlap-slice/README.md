# L0→L1 slice is GetOverlappingInputs of the hull, never the whole level

**Date:** 2026-09-07
**Primary sources:**
- RocksDB wiki *Choose Level Compaction Files* (EighteenZi mirror),
  `Choose-Level-Compaction-Files.md` in this directory. Step 6: find
  all files on Lo that overlap the selected inputs — not every file on Lo.
- facebook/rocksdb `db/compaction/compaction_picker_level.cc`
  (`compaction_picker_level.cc` in this directory).
  `LevelCompactionBuilder::SetupOtherInputsIfNeeded` calls
  `GetOverlappingInputs(output_level_, smallest, largest)` on the hull
  of the start-level files.

## What the source actually says

Leveled compaction's output-level inputs are the files whose key ranges
intersect the selected inputs' hull. Universal style is the opposite
("compact the entire range in one shot"). Absorbing a whole disjoint L1
on every L0→L1 job is the as-is mutant this pair refuses.

## Used this turn

Catalog pair `leveling_pick` (`data_fate`, entry `pick_l0_to_l1`):
Verus `overlapping_prefix` + `lemma_prefix_excludes` on production
`crates/pedradb-core/src/leveling.rs` (`cfg(verus_keep_ghost)`). The
rustc `pick_l0_to_l1` body stays last-wins (`Vec<u8>`). Pair
`leveling_pushdown` keeps its twin until its turn.

## Not claimed

RocksDB `ExpandInputsToCleanCut` (adjacent same-user-key files). Dump
of `prepare_l0_compact`. “somos seL4”.
