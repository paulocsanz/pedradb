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

## RFC-0035 — tabela completa vs F_FULLFSYNC (`58084ba`, 2026-08-16)

Mesmos knobs: 4096/2000 zipfian 1 KB, `ROCKS_PARITY_FULL_SYNC=1`, suíte `ycsb,deps` **na mesma run**. Peer = Rocks `sync` + `F_FULLFSYNC` nos `*.log`. Raw: [tikv-ycsb-0035-fullsync](tikv-ycsb-0035-fullsync/).

`slower` = Rocks_FF qps / Pedra qps. Teto 2× = slower ≤ 2. Teto 1.1× (RFC-0034) = slower ≤ 1.1.

| shape | Pedra qps | p50 | p95 | p99 | Rocks FF qps | p50 | p95 | p99 | slower | ≤2× |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|:---:|
| ycsb_a | 340 | 3.75 ms | 7.34 ms | 14.19 ms | 455 | 3.68 ms | 5.09 ms | 6.97 ms | **1.34×** | sim |
| ycsb_b | 2 814 | 2.3 µs | 4.43 ms | 7.01 ms | 3 644 | 2.7 µs | 3.77 ms | 5.45 ms | **1.29×** | sim |
| ycsb_c | 507 867 | 0.5 µs | 8.7 µs | 13 µs | 593 303 | 1.2 µs | 2.9 µs | 5.7 µs | **1.17×** | sim |
| ycsb_d | 2 162 | 2.3 µs | 4.18 ms | 8.35 ms | 3 819 | 2.6 µs | 3.88 ms | 5.26 ms | **1.77×** | sim |
| ycsb_e | 984 | 91 µs | 4.91 ms | 15.56 ms | 1 166 | 17 µs | 204 µs | 18.80 ms | **1.19×** | sim |
| ycsb_f | 301 | 4.12 ms | 8.63 ms | 22.05 ms | 284 | 3.94 ms | 14.15 ms | 22.13 ms | **0.94×** | sim |
| deps_apply_batch | 43.7 | 10.14 ms | 45.03 ms | 169.92 ms | 84.8 | 10.10 ms | 20.86 ms | 23.89 ms | **1.94×** | sim |
| deps_mvcc_latest | 4 314 | 1.5 µs | 125 µs | 1.43 ms | 138 058 | 5.5 µs | 13 µs | 35 µs | **32×** | não |
| deps_scan | 121 723 | 0.6 µs | 18 µs | 32 µs | 102 789 | 5.3 µs | 12 µs | 31 µs | **0.84×** | sim |
| deps_raftlog | 77.4 | 4.99 ms | 10.00 ms | 132.94 ms | 115.6 | 9.02 ms | 12.31 ms | 15.17 ms | **1.49×** | sim |
| deps_cache_overwrite | 107 | 4.96 ms | 16.90 ms | 157.16 ms | 104 | 9.91 ms | 13.18 ms | 17.08 ms | **0.97×** | sim |

**10/11 ≤2×. 3/11 ≤1.1×.** O único qps fora do 2× é `deps_mvcc_latest` nesta run combinada: p50 **1.5 µs** (mais rápido que Rocks 5.5 µs) mas qps 4.3k porque o p99/max (1.4 ms / 112 ms) puxa a média — LSM depois do YCSB (1 SST, cache frio no primeiro toque). A remesura **deps-only** do P1.3 (`rfc0035-p13g`) no mesmo commit: MVCC **116k / 0.56×** vs FF 207k (≤2×) e scan **246k / 1.16×**.

## RFC-0035 — tabela completa após seek O(log N) no get (2026-08-16)

Mesmos knobs, **mesma run ycsb+deps no mesmo LSM** (não isolámos o deps). Causa do 32×: `point_in_blocks` andava o índice esparso inteiro (1 L1 grande depois do compact YCSB+apply). Scan já usava `partition_point`; o get não. Peer = Rocks `F_FULLFSYNC`. Raw: [tikv-ycsb-0035-fullsync](tikv-ycsb-0035-fullsync/).

`slower` = Rocks_FF qps / Pedra qps. Teto 2× = slower ≤ 2. Teto 1.1× (RFC-0034) = slower ≤ 1.1.

| shape | Pedra qps | p50 | p95 | p99 | Rocks FF qps | p50 | p95 | p99 | slower | ≤2× |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|:---:|
| ycsb_a | 421 | 3.81 ms | 5.12 ms | 8.95 ms | 483 | 3.66 ms | 4.26 ms | 5.23 ms | **1.15×** | sim |
| ycsb_b | 3 960 | 1.0 µs | 3.85 ms | 5.00 ms | 4 098 | 1.4 µs | 3.88 ms | 4.98 ms | **1.03×** | sim |
| ycsb_c | 927 196 | 0.4 µs | 3.7 µs | 6.8 µs | 1 171 217 | 0.7 µs | 1.5 µs | 2.2 µs | **1.26×** | sim |
| ycsb_d | 4 287 | 0.9 µs | 3.82 ms | 4.88 ms | 4 650 | 1.4 µs | 3.86 ms | 4.07 ms | **1.08×** | sim |
| ycsb_e | 3 797 | 50 µs | 232 µs | 4.56 ms | 4 810 | 9.2 µs | 41 µs | 4.36 ms | **1.27×** | sim |
| ycsb_f | 431 | 3.91 ms | 5.00 ms | 8.92 ms | 450 | 3.88 ms | 4.97 ms | 6.27 ms | **1.04×** | sim |
| deps_apply_batch | 68.3 | 9.08 ms | 31.24 ms | 90.89 ms | 82.6 | 8.11 ms | 28.15 ms | 35.63 ms | **1.21×** | sim |
| deps_mvcc_latest | 298 691 | 0.7 µs | 14 µs | 18 µs | 272 898 | 3.2 µs | 6.3 µs | 8.5 µs | **0.91×** | sim |
| deps_scan | 346 658 | 0.4 µs | 12 µs | 19 µs | 250 820 | 3.5 µs | 5.5 µs | 9.4 µs | **0.72×** | sim |
| deps_raftlog | 123 | 5.03 ms | 13.28 ms | 93.09 ms | 171 | 4.12 ms | 8.11 ms | 10.11 ms | **1.39×** | sim |
| deps_cache_overwrite | 123 | 5.01 ms | 16.49 ms | 103.76 ms | 180 | 4.09 ms | 8.11 ms | 10.06 ms | **1.47×** | sim |

**11/11 ≤2×. 5/11 ≤1.1×** (b, d, f, mvcc, scan). RFC-0034 (1.1× all-shapes) continua aberto. Probe MVCC (mesmo 1 L1 / 471 SST fallback / 878 decodes que o 32×): encode 45 ns/op, last 1.5 µs, get **1.3 µs** (era 205 µs). WAL `sync_all` inalterado. Adversarial sem editar asserção.

## RFC-0035 — vs Rocks padrão do TiKV (`fdatasync`, 2026-08-16)

Mesmos knobs, run `ycsb,deps` combinada, `4ec565f`. Pedra = WAL `File::sync_all` (`F_FULLFSYNC` neste Mac). Peer = Rocks `WriteOptions.sync=true` **sem** `ROCKS_PARITY_FULL_SYNC` — `librocksdb-sys` desta build não define `HAVE_FULLFSYNC`, então o syscall é `fdatasync`. É a classe do TiKV `sync-log=true` em Linux (não é cluster 3-nós). Raw: [tikv-ycsb-0035-fdatasync](tikv-ycsb-0035-fdatasync/).

`slower` = Rocks_fd qps / Pedra qps. **Não é gate.** 2× de escrita vs esta coluna, mantendo G1, é fisicamente impossível (~4 ms `F_FULLFSYNC` vs ~30 µs `fdatasync`). Leituras (C, MVCC, scan) não pagam sync — mesmo trabalho que vs FF.

Seed 4096×1 KB: Pedra 17.5 s · Rocks fdatasync 0.2 s.

| shape | paga WAL | Pedra qps | p50 | p95 | p99 | Rocks fd qps | p50 | p95 | p99 | slower |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ycsb_a | sim | 431 | 3.74 ms | 5.10 ms | 8.09 ms | 15 730 | 27 µs | 132 µs | 483 µs | **37×** |
| ycsb_b | 5% | 4 237 | 0.9 µs | 3.82 ms | 4.12 ms | 238 270 | 0.8 µs | 32 µs | 86 µs | **56×** |
| ycsb_c | não | 900 411 | 0.4 µs | 3.5 µs | 5.8 µs | 1 249 219 | 0.7 µs | 1.3 µs | 1.8 µs | **1.39×** |
| ycsb_d | 5% | 4 485 | 0.9 µs | 3.75 ms | 4.10 ms | 251 302 | 0.8 µs | 29 µs | 82 µs | **56×** |
| ycsb_e | 5% | 3 870 | 49 µs | 238 µs | 4.51 ms | 21 122 | 8.0 µs | 37 µs | 142 µs | **5.5×** |
| ycsb_f | sim | 432 | 3.83 ms | 4.99 ms | 8.91 ms | 37 544 | 27 µs | 84 µs | 125 µs | **87×** |
| deps_apply_batch | sim | 72.4 | 8.07 ms | 30.90 ms | 97.42 ms | 5 017 | 186 µs | 269 µs | 407 µs | **69×** |
| deps_mvcc_latest | não | 337 909 | 0.6 µs | 13 µs | 17 µs | 261 131 | 3.2 µs | 6.5 µs | 9.0 µs | **0.77×** |
| deps_scan | não | 353 360 | 0.4 µs | 12 µs | 19 µs | 270 911 | 3.5 µs | 4.7 µs | 5.3 µs | **0.77×** |
| deps_raftlog | sim | 145 | 4.05 ms | 12.54 ms | 89.58 ms | 3 965 | 123 µs | 220 µs | 805 µs | **27×** |
| deps_cache_overwrite | sim | 148 | 4.12 ms | 13.17 ms | 104.53 ms | 28 141 | 29 µs | 60 µs | 93 µs | **191×** |

Leituras puras: C **1.39×**; MVCC e scan **mais rápidos** que o Rocks fd (0.77×). Escritas 27–191× — o piso é o syscall, não o LSM. Gate oficial continua vs `F_FULLFSYNC` (tabela acima).

## RFC-0036 — WAL `fdatasync` + L0-only compact vs Rocks TiKV (2026-08-16)

Pedra no Ok chama `fdatasync(2)` (`pedradb-posix`). Auto-compact promove **só L0 → L1 novo** (não reescreve o L1 existente). CHANGELOG interval 0: auto-flush não reescreve o feed (flush explícito / close ainda persistem; reopen reconstrói do SST). WAL **ainda sinca antes do Ok**. Adversarial sem editar asserção.

Mesmos knobs, ycsb+deps combinada. Peer = Rocks `fdatasync`. Raw: [tikv-ycsb-0036-fdatasync](tikv-ycsb-0036-fdatasync/).

`slower` = Rocks_fd / Pedra. Teto 2× = slower ≤ 2.

| shape | Pedra qps | p50 | p95 | p99 | Rocks fd qps | p50 | p95 | p99 | slower | ≤2× |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|:---:|
| ycsb_a | 57 486 | 21 µs | 48 µs | 55 µs | 60 707 | 22 µs | 45 µs | 50 µs | **1.06×** | sim |
| ycsb_b | 343 220 | 0.4 µs | 28 µs | 49 µs | 410 604 | 0.7 µs | 24 µs | 42 µs | **1.20×** | sim |
| ycsb_c | 1 322 933 | 0.3 µs | 3.2 µs | 5.4 µs | 1 440 014 | 0.6 µs | 1.0 µs | 1.2 µs | **1.09×** | sim |
| ycsb_d | 397 562 | 0.4 µs | 24 µs | 45 µs | 435 106 | 0.7 µs | 23 µs | 42 µs | **1.09×** | sim |
| ycsb_e | 157 871 | 5.2 µs | 18 µs | 48 µs | 106 872 | 7.2 µs | 15 µs | 44 µs | **0.68×** | sim |
| ycsb_f | 53 345 | 26 µs | 51 µs | 57 µs | 55 971 | 25 µs | 48 µs | 53 µs | **1.05×** | sim |
| deps_apply_batch | 1 908 | 166 µs | 207 µs | 334 µs | 4 405 | 169 µs | 198 µs | 287 µs | **2.31×** | não |
| deps_mvcc_latest | 424 253 | 0.5 µs | 9.7 µs | 20 µs | 311 024 | 2.9 µs | 5.4 µs | 8.3 µs | **0.73×** | sim |
| deps_scan | 212 099 | 0.3 µs | 18 µs | 39 µs | 303 195 | 3.2 µs | 3.6 µs | 3.9 µs | **1.43×** | sim |
| deps_raftlog | 6 588 | 43 µs | 68 µs | 98 µs | 3 244 | 81 µs | 336 µs | 5.74 ms | **0.49×** | sim |
| deps_cache_overwrite | 36 653 | 27 µs | 32 µs | 35 µs | 25 619 | 26 µs | 50 µs | 59 µs | **0.70×** | sim |

**10/11 ≤2×.** apply p50 **empatado** com o Rocks (166 vs 169 µs); qps 2.31× por um compact L0 no writer (max 221 ms). G6 = sem thread. WAL sincado antes do Ok.

## RFC-0037 P0.2 — L0 compact em streaming (2026-08-16)

K-way por bloco, sem `entries_cloned` do L0 inteiro. Mesmos knobs, peer Rocks fd. Raw: [tikv-ycsb-0037-streaming](tikv-ycsb-0037-streaming/). Oficial = **p03c** (Rocks apply limpo).

| shape | Pedra qps | p50 | p95 | p99 | Rocks fd qps | p50 | p95 | p99 | slower | ≤2× |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|:---:|
| ycsb_a | 53 182 | 20 µs | 55 µs | 73 µs | 42 338 | 20 µs | 51 µs | 84 µs | **0.80×** | sim |
| ycsb_b | 299 325 | 0.6 µs | 23 µs | 55 µs | 391 224 | 0.8 µs | 25 µs | 45 µs | **1.31×** | sim |
| ycsb_c | 1 153 181 | 0.3 µs | 3.5 µs | 5.7 µs | 1 364 684 | 0.7 µs | 1.0 µs | 1.3 µs | **1.18×** | sim |
| ycsb_d | 363 898 | 0.5 µs | 20 µs | 48 µs | 383 718 | 0.7 µs | 25 µs | 46 µs | **1.05×** | sim |
| ycsb_e | 143 163 | 5.5 µs | 22 µs | 49 µs | 102 758 | 7.5 µs | 25 µs | 51 µs | **0.72×** | sim |
| ycsb_f | 47 310 | 23 µs | 61 µs | 79 µs | 56 522 | 23 µs | 49 µs | 56 µs | **1.19×** | sim |
| deps_apply_batch | 1 835 | 179 µs | 298 µs | 3.07 ms | 5 576 | 174 µs | 209 µs | 329 µs | **3.04×** | não |
| deps_mvcc_latest | 411 727 | 0.5 µs | 9.8 µs | 20 µs | 302 113 | 2.9 µs | 5.8 µs | 9.4 µs | **0.73×** | sim |
| deps_scan | 211 553 | 0.4 µs | 18 µs | 38 µs | 292 440 | 3.3 µs | 4.0 µs | 4.3 µs | **1.38×** | sim |
| deps_raftlog | 5 136 | 46 µs | 88 µs | 251 µs | 4 568 | 83 µs | 182 µs | 2.90 ms | **0.89×** | sim |
| deps_cache_overwrite | 36 693 | 26 µs | 40 µs | 56 µs | 30 649 | 24 µs | 49 µs | 57 µs | **0.84×** | sim |

**Ainda 10/11.** Apply qps ≈ 0036 (1 835 vs 1 908); max 221 ms → 105 ms. Três runs: Pedra apply 1 835–1 949; Rocks apply 2 791 / 3 623 / **5 576**. Só a terceira é limpa (Rocks max 0.55 ms). Streaming não tira o rewrite do put — P0.3 11/11 não fechou.

## RFC-0033 remesure (2026-08-15, deps-only)

Same knobs (4096/2000, zipfian, 1 KB). Compat only — no Rocks peer in this slice. After apply (64k txns, batch=32).

| shape | after 0032 (`5cf09a9`) | P0.1/P0.2 | P0.3 (lazy + block cache) | floor 2× |
|---|---:|---:|---:|---:|
| deps_mvcc_latest | 124 qps / p50 1.9 ms | 899 / 0.90 ms | **1,190 / 0.75 ms** | 37k |
| deps_scan | 1,024 qps / p50 0.97 ms | 333 / 1.20 ms | **2,778 / 0.35 ms** | 6.5k |

Scan ~8× vs the P0.2 dip; still ~0.43× of the 6.5k floor (overlapping L0 per seek). MVCC still ≪ 37k.

Guarantees: WAL `sync_all` unchanged. Per-layer scan cap **not** shipped (would hide live keys after a deleted prefix). Adversarial assertions unchanged.
