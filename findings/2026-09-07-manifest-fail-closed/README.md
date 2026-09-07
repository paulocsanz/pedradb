# MANIFEST fail-closed: refuse damage / missing SST, never silent scan

**Date:** 2026-09-07
**Primary sources:**
- Memgraph #3785 (opened 2026-02-12): truncated RocksDB MANIFEST on restart
  reports `Corruption` then SIGSEGV (exit 139) instead of a clean refuse.
  Raw: `memgraph-3785.md`.
- RocksDB #13624 (opened 2025-05-19): `VersionBuilder` treats an inconsistent
  SST delete (file not in the LSM tree) as `Corruption` on `DB::Open`, not a
  directory scan. Raw: `rocksdb-13624.md`.

## What the sources actually say

A torn or inconsistent SST inventory is not “repair by scanning the dir”.
Memgraph’s truncated MANIFEST is already `Corruption` from RocksDB; the bug
is that the process then dies with SIGSEGV instead of refusing the open.
RocksDB’s VersionBuilder (since #6901) fail-closes on MANIFEST/LSM inventory
inconsistency (`Cannot delete table file #N … since it is not in the LSM
tree`). Neither source authorizes serving a reconstructed inventory that was
never committed.

## Used this turn

Catalog pair `manifest_recover` (`data_fate`, entry `sst_recover_action`):
`Corrupt` or `Inventory`+`Missing(n)` ⇒ `RefuseOpen`. AS-IS scans on damage
(resurrects GC’d files). Production `manifest_kernel.rs` is now the Verus
term (`single_artifact`). Lemmas `lemma_damaged_inventory_never_scans`,
`lemma_missing_listed_sst_refuses`, and `lemma_mutant_scan_resurrects_gc`
are the second possibility. Pair `first_install` keeps the twin until its
turn.

## Not claimed

Dump of `db.rs` `recover_ssts` glue. RocksDB VersionBuilder itself. Memgraph
SIGSEGV fix. “somos seL4”.
