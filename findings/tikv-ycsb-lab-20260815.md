# TiKV-documented YCSB mixes — engine pair (2026-08-15)

**Not a TiKV cluster.** Official TiKV bench is `go-ycsb` → 3-node RawKV (PD + raftstore + gRPC). We cannot swap Pedra into that stack (compat is not drop-in; see `docs/rocksdb-compat.md`). This run is the **same mixes TiKV documents**, on the rocks-parity pair: `rocksdb-compat` (pedradb-core) vs real RocksDB (`rocksdb` 0.22), single node, single client, identical op schedule.

## Mix provenance

| shape | TiKV / Yahoo / go-ycsb | This run |
|---|---|---|
| ycsb_a | 50/50 update-heavy (session store) | same |
| ycsb_b | 95/5 read-mostly | same |
| ycsb_c | 100% read | same |
| ycsb_d | read-latest + insert | same |
| ycsb_e | 95% short scan + 5% insert | same (scan window 25) |
| ycsb_f | 50% RMW | same |
| deps_* | TiKV engine path (raftstore apply, MVCC SeekForPrev, raftdb) | our suite |

Knobs: `recordcount=4096`, `operationcount=2000`, payload **1 KB** (Yahoo default 10×100), **zipfian** (Yahoo wiki / TiKV-docs; go-ycsb checked-in files say uniform). Single thread.

Reproduce: `scripts/tikv_ycsb_parity_v0.sh`.

## Lab numbers

Seed 4096×1 KB: Pedra 20.8 s · Rocks fdatasync 0.5 s · Rocks F_FULLFSYNC 24.6 s.

| shape | Pedra qps | Rocks fdatasync | ratio | Rocks F_FULLFSYNC | ratio |
|---|---:|---:|---:|---:|---:|
| ycsb_a | 329 | 18,437 | 0.018 | 339 | **0.97** |
| ycsb_b | 2,096 | 113,784 | 0.018 | 3,149 | **0.67** |
| ycsb_c | 14,909 | 460,454 | 0.032 | 373,178 | 0.040 |
| ycsb_d | 2,219 | 106,720 | 0.021 | 2,836 | **0.78** |
| ycsb_e | 28 | 36,362 | 0.001 | 3,015 | 0.009 |
| ycsb_f | 345 | 8,735 | 0.040 | 332 | **1.04** |
| deps_apply_batch | 43 | 1,194 | 0.036 | 61 | **0.70** |
| deps_mvcc_latest | 3.2 | 107,259 | 0.000 | 74,605 | 0.000 |
| deps_scan | 13 | 7,352 | 0.002 | 12,961 | 0.001 |
| deps_raftlog | 87 | 2,507 | 0.035 | 117 | **0.75** |
| deps_cache_overwrite | 115 | 6,159 | 0.019 | 101 | **1.14** |

Pedra p50: ycsb_a 3.7 ms (one WAL `sync_all`) · ycsb_c 0.020 ms · ycsb_e 17 ms · deps_mvcc_latest **292 ms**.

## How to read this vs TiKV's published 200k OPS

TiKV docs (3-node, 10M records, GO YCSB RawKV): ~212k point-get (C), ~43k update (A). That is **distributed + multi-client + fdatasync-class WAL**, not this laptop, not this keyspace. Do not put our 329 qps next to 43k as "Pedra vs TiKV".

What this run *does* say:

1. **Against how Rocks actually syncs in this rust build (fdatasync):** Pedra is ~50× slower on write mixes, ~30× on point-get, **100–1000×** on scans/MVCC. That is the number a TiKV-shaped deployment would feel if it kept Pedra's `F_FULLFSYNC` and eager iterators.
2. **Against the same durability class (`F_FULLFSYNC`):** write mixes are **0.67–1.14×** (several already faster). The remaining hole is **reads that use the iterator** (ycsb_e, deps_mvcc, deps_scan) and point-get (~25×).
3. zipfian + 1 KB did not change the write story vs the earlier uniform/100 B run. It **destroyed** MVCC-latest (3 qps, p50 292 ms) because the eager iterator now copies a 4k×2-version 1 KB CF every seek.

Published TiKV cluster numbers are not a Pedra target until there is a TiKV on Pedra. This pair is the honest engine-level answer today.

## RFC-0033 P0.1/P0.2 remesure (2026-08-15, deps-only)

Same knobs (4096/2000, zipfian, 1 KB). Compat only — no Rocks peer in this slice. After apply (64k txns, batch=32).

| shape | after 0032 (`5cf09a9`) | after 0033 P0.1/P0.2 | note |
|---|---:|---:|---|
| deps_mvcc_latest | 124 qps / p50 1.9 ms | **899 qps / p50 0.90 ms** | `last_under_prefix`; still ≪ floor 37k |
| deps_scan | 1,024 qps / p50 0.97 ms | 333 qps / p50 1.20 ms | p50 similar; qps tail (P0.3 still open) |

Smoke 512/200 (smaller LSM): mvcc 5.4k qps / p50 0.15 ms · scan 9.1k qps / p50 0.077 ms.

Guarantees: WAL `sync_all` unchanged. Per-layer scan cap **not** shipped (would hide live keys after a deleted prefix). Adversarial assertions unchanged.
