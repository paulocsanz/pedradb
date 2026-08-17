# rocksdb-compat on ConcurrentDb (2026-08-17)

`rocksdb-compat::DB` now wraps `pedradb-core::ConcurrentDb` instead of
`Mutex<Db>`. Writes join the Rocks-style write group; a lone client takes
the single-writer fast path (`apply_batch_with`, no mpsc hop). Reads take
`RwLock` read guards. `open_cf_with_env` stays worker-free (FailingEnv).

## Engine changes that showed up in the numbers

1. **SST write no longer holds a Db lock.** `ConcurrentDb::flush` /
   `drain_imm_once` used to keep a *read* guard across `write_sst` — that
   blocked every put for the whole SST I/O. Now: snapshot `(env, dir, sync)`
   under a short write lock, write the file with no guard.
2. **Idle compact worker is a read check.** The 20 ms poll used to take a
   write lock just to see `has_imm` / L0 count, stalling YCSB C/scan.
3. **Count cursor is a concrete BTree range** (no `Box<dyn Iterator>` on
   the common no-tombstone path); empty mem layers are skipped.

## Honesty (G8)

This box is noisy (OrbStack / Terminal). **QPS ratios are not a claim** —
one 10–200 ms spike in a 2 ms YCSB C window wrecks `n/wall`. Judge:

- **p50** of the same run (below) — Pedra often *faster* than Rocks on
  the typical op.
- **one quiet run** (run5) for absolute qps.

We do **not** claim 11/11 qps > Rocks on this machine. Scan p95 is still
~4× Rocks on a miss (merge of several L0+L1 files). That is the remaining
gap, not the lock type.

### Quiet run (run5) — qps

| shape | compat qps | rocks qps | ratio |
|---|---:|---:|---:|
| ycsb_a | 41 002 | 27 522 | **1.49** |
| ycsb_b | 246 827 | 316 907 | 0.78 |
| ycsb_c | 2 054 707 | 1 276 324 | **1.61** |
| ycsb_d | 158 985 | 335 224 | 0.47 |
| ycsb_e | 101 872 | 88 717 | **1.15** |
| ycsb_f | 24 875 | 27 309 | 0.91 |
| deps_apply_batch | 1 776 | 2 790 | 0.64 |
| deps_mvcc_latest | 386 582 | 262 096 | **1.48** |
| deps_scan | 154 707 | 252 255 | 0.61 |
| deps_raftlog | 3 036 | 1 688 | **1.80** |
| deps_cache_overwrite | 12 960 | 22 748 | 0.57 |

### Same run — p50 ms (typical op)

| shape | compat p50 | rocks p50 | faster? |
|---|---:|---:|---|
| ycsb_a | 0.0250 | 0.0267 | yes |
| ycsb_b | 0.0006 | 0.0008 | yes |
| ycsb_c | 0.0003 | 0.0007 | yes |
| ycsb_d | 0.0005 | 0.0008 | yes |
| ycsb_e | 0.0040 | 0.0081 | yes |
| ycsb_f | 0.0273 | 0.0305 | yes |
| deps_apply_batch | 0.3496 | 0.2243 | no |
| deps_mvcc_latest | 0.0008 | 0.0034 | yes |
| deps_scan | 0.0003 | 0.0037 | yes (cache); p95 0.022 vs 0.005 |
| deps_raftlog | 0.1638 | 0.1147 | no |
| deps_cache_overwrite | 0.0318 | 0.0325 | yes |

Durability class: both sides `fdatasync` before Ok
(`WriteOptions.sync=true` / Pedra default). Single client.

Raw: `run5/compare/compare_report.json`.
