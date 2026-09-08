# Compaction pick merges the lowest non-empty level exactly one level down

**Date:** 2026-09-07
**Primary source:** this kernel’s own crash-dictionary contract
(`docs/formal/crash-dictionary.md`, compaction section) plus RocksDB
leveled compaction’s “pick a file and merge it into the next level”
(already persisted under `findings/2026-09-07-leveling-pick-overlap-slice/`).

## What the source actually says

If any level below max has files, the plan is Merge `{from: l, to: l+1}`.
GcRewriteMax only when the tree is already at max and GC was requested.
The as-is mutant is always NoOp.

## Used this turn

Catalog pair `compact_decision` (`data_fate`, entry `compact_pick`):
Verus `compact_pick_spec` on production `compact_kernel.rs`. rustc
4-arg body (`max_level` unused in the decision) stays last-wins.
`compact_retention` / `pin_gc` keep twins.

## Not claimed

Dump of `compact_with_ssts_only`. “somos seL4”.
