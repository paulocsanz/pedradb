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

## RFC-0034 remesure — full pair vs F_FULLFSYNC (`af2c2d5`/`fbe39bf`)

`scripts/tikv_ycsb_parity_v0.sh` + `ROCKS_PARITY_FULL_SYNC=1`, 4096/2000 zipfian 1 KB. Raw: [tikv-ycsb-0034-fullsync](tikv-ycsb-0034-fullsync/).

`slower` = Rocks_FF qps / Pedra qps. Alvo 1.1× = slower ≤ 1.1. **Nenhum shape passa.**

| shape | Pedra qps | Pedra p50 | Rocks FF qps | Rocks p50 | ratio | slower |
|---|---:|---:|---:|---:|---:|---:|
| ycsb_a | 401 | 3.58 ms | 449 | 3.60 ms | 0.893 | **1.12×** |
| ycsb_b | 3 385 | 6.5 µs | 4 548 | 1.5 µs | 0.744 | **1.34×** |
| ycsb_c | 152 728 | 5.7 µs | 1 224 864 | 0.7 µs | 0.125 | **8.0×** |
| ycsb_d | 3 789 | 12 µs | 4 770 | 1.9 µs | 0.794 | **1.26×** |
| ycsb_e | 2 280 | 0.19 ms | 4 942 | 9 µs | 0.461 | **2.17×** |
| ycsb_f | 391 | 3.76 ms | 464 | 3.85 ms | 0.843 | **1.19×** |
| deps_apply_batch | 66 | 9.09 ms | 83 | 8.78 ms | 0.798 | **1.25×** |
| deps_mvcc_latest | 2 465 | 0.37 ms | 170 195 | 4.2 µs | 0.014 | **69×** |
| deps_scan | 5 532 | 0.18 ms | 192 201 | 3.5 µs | 0.029 | **35×** |
| deps_raftlog | 105 | 4.92 ms | 189 | 4.16 ms | 0.556 | **1.80×** |
| deps_cache_overwrite | 124 | 4.54 ms | 218 | 4.08 ms | 0.567 | **1.76×** |

Do not read the `07bd443` A/F/overwrite “≥ 1×” lines as current. This peer is faster; Pedra is not inside 1.1× on any row.

## RFC-0033 / 2× on MVCC + deps_scan (2026-08-16)

Same knobs, deps-only after apply, Rocks FF peer ~160k / ~165k qps.

| shape | `eaa4adf` (0034 remesura) | after mem-hit `last_under_user_prefix` + no block clone | Rocks FF | slower | 2×? |
|---|---:|---:|---:|---:|---|
| deps_mvcc_latest | 2 465 / 69× | **9 006 / p50 77 µs** | 159 790 / 5.4 µs | **18×** | não |
| deps_scan | 5 532 / 35× | **6 117 / p50 162 µs** | 164 889 / 5.7 µs | **27×** | não |

WAL `sync_all` unchanged. `last_under_user_prefix` only skips SST when newest mem already has a live key of that user (MVCC suffix). Tombstone of the latest still falls back to full `last_under_prefix` + lookup (tested). Scan iterates `Arc` blocks (no 4 KB clone). Residual: overlapping L0 last_visible/merge when the key is not in mem.

## RFC-0033 remesure (2026-08-15, deps-only)

Same knobs (4096/2000, zipfian, 1 KB). Compat only — no Rocks peer in this slice. After apply (64k txns, batch=32).

| shape | after 0032 (`5cf09a9`) | P0.1/P0.2 | P0.3 (lazy + block cache) | floor 2× |
|---|---:|---:|---:|---:|
| deps_mvcc_latest | 124 qps / p50 1.9 ms | 899 / 0.90 ms | **1,190 / 0.75 ms** | 37k |
| deps_scan | 1,024 qps / p50 0.97 ms | 333 / 1.20 ms | **2,778 / 0.35 ms** | 6.5k |

Scan ~8× vs the P0.2 dip; still ~0.43× of the 6.5k floor (overlapping L0 per seek). MVCC still ≪ 37k.

Guarantees: WAL `sync_all` unchanged. Per-layer scan cap **not** shipped (would hide live keys after a deleted prefix). Adversarial assertions unchanged.
