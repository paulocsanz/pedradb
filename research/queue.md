# Queue — waves (histórico)

**Canónico para “o que fazer agora”:** [`PLANO.md`](PLANO.md).
Este ficheiro guarda as waves completas.

**Updated:** 2026-08-15
**Policy:** five well-read papers beat fifty unread PDFs. Pop from the top.

Wave 0 is "close the loop on decisions we already made or are about to make." Wave 1 is the 2023–2026 LSM theory we have not read. Wave 2 is layers on top.

## Wave 0 — foundations we keep citing (do these first)

| Order | ID | Paper | Why now | PDF hint |
|------:|----|-------|---------|----------|
| 1 | R010 | RocksDB Experience (FAST'21) | **ficha D4** — L9 + L25–L27 | `research/fontes/R010_Dong_2021_RocksExperience.pdf` |
| 2 | R043 | FoundationDB (SIGMOD'21) | **ficha D4** — L29 REFUSE role-split | `research/fontes/R043_Zhou_2021_FoundationDB.pdf` |
| 3 | R044 | FDB Record Layer (SIGMOD'19) | **ficha D4** — L31 SHIP / L32–L34 | `research/fontes/R044_Chrysafis_2019_RecordLayer.pdf` |
| 4 | R045 | CockroachDB (SIGMOD'20) | **ficha D4** — L39 MEASURE / L40 REFUSE | `research/fontes/R045_Taft_2020_CockroachDB.pdf` |
| 5 | R012 | LSM compaction design space (VLDB'21) | **ficha D4** — L35 MEASURE / L36–L37 REFUSE | `research/fontes/R012_Sarkar_2021_LSMCompaction.pdf` |
| 6 | R041 | Percolator (OSDI'10) | **ficha D4** — L22/L23 REFUSE | `docs/references/percolator-osdi2010.pdf` |
| 7 | R005 | WiscKey (FAST'16) | **already local** — fichar; GC still open | `docs/references/wisckey-fast2016.pdf` |
| 8 | R006 | Monkey (SIGMOD'17) | **ficha D4** — L1 SHIP / L2 MEASURE | `docs/references/monkey-sigmod2017.pdf` |
| 9 | R007 | Dostoevsky (SIGMOD'18) | **ficha D4** — L3 REFUSE confirmed | `docs/references/dostoevsky-sigmod2018.pdf` |
| 10 | R048 | TiDB (VLDB'20) | **ficha D4** — L8 SHIP / L24 REFUSE | `docs/references/tidb-raft-htap-vldb2020.pdf` |

## Wave 1 — recent LSM we must not ignore

| Order | ID | Paper | Why now | PDF hint |
|------:|----|-------|---------|----------|
| 11 | R014 | Endure (VLDB'22) | **ficha D4** — L12; always leveling | `research/fontes/R014_Huynh_2022_Endure.pdf` |
| 12 | R013 | Spooky (VLDB'22) | **ficha D4** — L11 after L35 | `research/fontes/R013_Dayan_2022_Spooky.pdf` |
| 13 | R016 | ADOC (FAST'23) | **ficha D4** — L41 overflow; L42 tuner REFUSE | `research/fontes/R016_Yu_2023_ADOC.pdf` |
| 14 | R017 | SILK (ATC'19) | **ficha D4** — L41 flush>L0 | `research/fontes/R017_Balmau_2019_SILK.pdf` |
| 14b | R031 | Lethe (SIGMOD'20) | **ficha D4** — L38 FADE; L43 KiWi REFUSE | `research/fontes/R031_Sarkar_2020_Lethe.pdf` |
| 15 | R023 | Disco (SIGMOD'25) | compact multi-run index | ACM / author |
| 16 | R028 | How to Grow an LSM-tree (SIGMOD'25) | vertical vs horizontal growth | arXiv `2504.17178` |
| 17 | R027 | EcoTune / compaction policies (SIGMOD'25) | when to compact | Tsinghua PDF |
| 18 | R029 | SplinterDB (ATC'20) | NVMe-first alternative | usenix ATC'20 |
| 19 | R039 | LSM-KVS survey (2025) | map of 2020–2025; do not treat as primary | arXiv `2507.09642` |
| 20 | R040 | TurtleKV (VLDB'26) | newest hybrid claim | arXiv `2509.10714` |

## Wave 2 — recovery, isolation, layers

Start here after wave 0+1 have at least 8 fichas.

| ID | Paper |
|----|-------|
| R082 | ARIES |
| R088 | All File Systems Are Not Created Equal |
| R089 | Protocol-Aware Recovery |
| R083 | Critique of ANSI SQL Isolation |
| R085 | SSI in PostgreSQL |
| R052 | Raft |
| R053 | Calvin |
| R042 | Spanner |
| R046 | CRDB multi-region |
| R066 | HTAP survey (already local — fichar) |

## Done

| ID | Tier | Data |
|----|------|------|
| R005 WiscKey | D4 | 2026-08-14 |
