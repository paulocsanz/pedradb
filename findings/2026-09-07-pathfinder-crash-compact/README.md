# Compact crash window: never drop until the replacement is durable

**Date:** 2026-09-07
**Primary source:** Gu, Neal, et al., *Scalable and Accurate Application-Level Crash-Consistency Testing via Representative Testing* (Pathfinder), arXiv:2503.01390 (Mar 2025). PDF: `2503.01390.pdf`.

## What the paper actually does

Pathfinder reduces crash-state space by clustering correlated crash states (update-behavior heuristic). Evaluated on POSIX KV stores including RocksDB: WAL + memtable + **background compaction** into SST. The crash bugs they hunt are “compact dropped a prefix the recovery still needs.”

## Used this turn

Catalog pair `compact_unleft` (`data_fate`): `compact_through_unleft` is the Pedra analogue on the raft log — do not compact through an applied still-active joint (`old != new`, no later leave). AS-IS drops the joint (RFC-0096/0100 hole). The production file is now the Verus term (`single_artifact`). Lemma `lemma_as_is_compacts_past_unleft` is the second possibility.

Not LSM SST compact (core `compact_kernel.rs`); this is membership-log compact.

## Not claimed

Pathfinder campaign = ∀ crash states. Dump of `store/src/lib.rs` compact glue.
