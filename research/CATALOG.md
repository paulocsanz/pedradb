# Catalog — 100 papers for PedraDB

**Updated:** 2026-08-14
**Machine copy:** [`catalog.tsv`](catalog.tsv) (same ids)
**How ranked:** [`sources/HOW-THIS-LIST-WAS-BUILT.md`](sources/HOW-THIS-LIST-WAS-BUILT.md)
**Norma:** [`QUALIDADE.md`](QUALIDADE.md) · **vai:** [`PLANO.md`](PLANO.md) · **ledger:** [`LEDGER.md`](LEDGER.md)

This is a **relevance ranking for this repo**, not a global top-100. Recency is weighted (40 of 100 are 2020–2026) but canonical theory stays because we keep re-deriving it.

**Status today:** 13 PDFs em `docs/references/`. **1 ficha D4** (R005). O resto do catálogo continua hipótese até ficha.

Legend: **C** = canonical · **R** = recent (2020–2026) · status `listed` | `have-pdf` | `ficha` | `blocked`

## Ranked 100

| # | ID | Year | Venue | Layer | Rec | Status | Title | Pedra hook |
|--:|----|-----:|-------|-------|-----|--------|-------|------------|
| 1 | R001 | 1996 | Acta Inf. | engine | C | listed | The Log-Structured Merge-Tree | the tree we are |
| 2 | R002 | 2006 | OSDI | dist | C | listed | Bigtable | tablet = range + local log |
| 3 | R003 | 2007 | SOSP | dist | C | listed | Dynamo | AP contrast; we are CP |
| 4 | R004 | 2012 | SIGMOD | engine | C | listed | bLSM | Bloom + stall scheduling |
| 5 | R005 | 2016 | FAST | engine | C | ficha | WiscKey | vlog; GC incremental OPEN |
| 6 | R006 | 2017 | SIGMOD | engine | C | have-pdf | Monkey | non-uniform Bloom FPR |
| 7 | R007 | 2018 | SIGMOD | engine | C | have-pdf | Dostoevsky | Lazy Leveling; **non-ship** |
| 8 | R008 | 2017 | SOSP | engine | C | listed | PebblesDB | fragmented LSM / guards |
| 9 | R009 | 2020 | FAST | engine | R | listed | RocksDB workloads at Facebook | real key/value sizes |
| 10 | R010 | 2021 | FAST | engine | R | listed | The RocksDB Experience | incumbent we are scored against |
| 11 | R011 | 2020 | CSUR | engine | R | listed | LSM-based Storage Techniques | pre-2020 map |
| 12 | R012 | 2021 | VLDB | engine | R | listed | LSM compaction design space | when / which / how much / layout |
| 13 | R013 | 2022 | VLDB | engine | R | listed | Spooky | granulated compact |
| 14 | R014 | 2022 | VLDB | engine | R | listed | Endure | robust tuning |
| 15 | R015 | 2024 | VLDBJ | engine | R | listed | Flexibility and robustness of LSM trees | Endure journal |
| 16 | R016 | 2023 | FAST | engine | R | listed | ADOC | stall = dataflow |
| 17 | R017 | 2019 | ATC | engine | C | listed | SILK | pacing / tail latency |
| 18 | R018 | 2018 | ATC | engine | C | listed | HashKV | vlog GC locality |
| 19 | R019 | 2020 | OSDI | engine | R | listed | Bourbon | learned SST index; likely refuse |
| 20 | R020 | 2021 | SIGMOD | ds | R | listed | Chucky | cuckoo filter for LSM |
| 21 | R021 | 2021 | arXiv | ds | R | listed | Ribbon filter | Rocks shipped it; we have Bloom |
| 22 | R022 | 2021 | FAST | engine | R | listed | REMIX | range amp |
| 23 | R023 | 2025 | SIGMOD | engine | R | listed | Disco | compact multi-run index |
| 24 | R024 | 2024 | SIGMOD | engine | R | listed | GRF | global range filter |
| 25 | R025 | 2024 | SIGMOD | engine | R | listed | Moose | per-level knobs |
| 26 | R026 | 2023 | SIGMOD | engine | R | listed | RusKey | device-aware LSM |
| 27 | R027 | 2025 | SIGMOD | engine | R | listed | Rethinking compaction policies | EcoTune / when |
| 28 | R028 | 2025 | SIGMOD | engine | R | listed | How to Grow an LSM-tree | vertical vs horizontal growth |
| 29 | R029 | 2020 | ATC | engine | R | listed | SplinterDB | NVMe-first alternative |
| 30 | R030 | 2023 | SIGMOD | engine | R | listed | SplinterDB and Maplets | compact policy on Bε-tree |
| 31 | R031 | 2020 | SIGMOD | engine | R | listed | Lethe | deletes that disappear |
| 32 | R032 | 2023 | ICDE | htap | R | have-pdf | Real-Time LSM-Trees | LASER; not 2nd primary |
| 33 | R033 | 2018 | SIGMOD | ds | C | listed | SuRF | range filter |
| 34 | R034 | 2020 | SIGMOD | ds | R | listed | Rosetta | range filter |
| 35 | R035 | 2019 | ATC | ds | C | listed | ElasticBF | hotness-aware Bloom RAM |
| 36 | R036 | 2017 | ATC | engine | C | listed | TRIAD | defer compact until overlap pays |
| 37 | R037 | 2019 | SOSP | engine | C | listed | KVell | no-compact NVMe contrast |
| 38 | R038 | 2018 | SIGMOD | engine | C | listed | FASTER | hybrid log / in-place hot |
| 39 | R039 | 2025 | arXiv | engine | R | listed | Rethinking LSM-KVS (survey) | 2020–25 map; not primary |
| 40 | R040 | 2026 | VLDB* | engine | R | listed | TurtleKV | newest hybrid; confirm camera-ready |
| 41 | R041 | 2010 | OSDI | layer | C | have-pdf | Percolator | OCC + oracle + 2PC on KV |
| 42 | R042 | 2012 | OSDI | dist | C | listed | Spanner | TrueTime we will not fake |
| 43 | R043 | 2021 | SIGMOD | dist | R | listed | FoundationDB | Montanha face + DST |
| 44 | R044 | 2019 | SIGMOD | layer | C | listed | FDB Record Layer | indexes as KV projections |
| 45 | R045 | 2020 | SIGMOD | layer | R | listed | CockroachDB | SQL + Multi-Raft + Pebble |
| 46 | R046 | 2022 | SIGMOD | dist | R | listed | Multi-region CockroachDB | declarative geo |
| 47 | R047 | 2025 | SIGMOD | dist | R | listed | CockroachDB Serverless | multi-tenant virt; not P0 |
| 48 | R048 | 2020 | VLDB | layer | R | have-pdf | TiDB (Raft HTAP) | learner ≠ voter |
| 49 | R049 | 2022 | VLDB | layer | R | listed | OceanBase | LSM inside distributed SQL |
| 50 | R050 | 2023 | VLDB | dist | R | listed | OceanBase Paetica | one binary, one or many nodes |
| 51 | R051 | 2022 | VLDB | engine | R | listed | Magma | LSM + segmented log |
| 52 | R052 | 2014 | ATC | dist | C | listed | Raft | `pedradb-raft` |
| 53 | R053 | 2012 | SIGMOD | dist | C | listed | Calvin | deterministic apply vs OCC |
| 54 | R054 | 2017 | SIGMOD | layer | C | listed | Amazon Aurora | the log *is* the database |
| 55 | R055 | 2016 | SIGMOD | layer | C | listed | Snowflake | disagg warehouse |
| 56 | R056 | 2026 | SIGMOD | dist | R | listed | Scalable leader leases | Multi-Raft leases |
| 57 | R057 | 2018 | OSDI | dist | C | listed | Akkio | placement vs shard count |
| 58 | R058 | 2013 | VLDB | layer | C | listed | F1 | SQL on a TX KV |
| 59 | R059 | 2024 | SIGMOD | engine | R | listed | CaaS-LSM | compact offload; later |
| 60 | R060 | 2021 | FAST | engine | R | listed | SpanDB | WAL on the fast device |
| 61 | R061 | 2010 | SoCC | test | C | listed | YCSB | name the workload |
| 62 | R062 | 2020 | SIGMOD | engine | R | listed | KV storage engines tutorial | shared vocab with Rocks |
| 63 | R063 | 2021 | SIGMOD | layer | R | listed | LogStore | multi-tenant log / stream |
| 64 | R064 | 2021 | ATC | engine | R | listed | DiffKV | hot/cold paths in LSM |
| 65 | R065 | 2022 | POMACS | engine | R | listed | Dremel | auto-tune Rocks; offline only |
| 66 | R066 | 2024 | arXiv | htap | R | have-pdf | HTAP Databases: A Survey | triangle doctrine |
| 67 | R067 | 2024 | VLDBJ | htap | R | listed | HTAP survey (Song et al.) | independent cross-check |
| 68 | R068 | 2022 | VLDB | htap | R | have-pdf | ByteHTAP | production point on the triangle |
| 69 | R069 | 2025 | VLDB | htap | R | have-pdf | veDB-HTAP | newest industry HTAP |
| 70 | R070 | 2023 | SIGMOD | htap | R | have-pdf | PolarDB-IMCI | planner routing |
| 71 | R071 | 2022 | ICDE | htap | R | have-pdf | Polynesia | PIM; **not** a Pedra dep |
| 72 | R072 | 2025 | ASPLOS | htap | R | have-pdf | PUSHtap | hardware; **not** a Pedra dep |
| 73 | R073 | 2025 | FAST | htap | R | have-pdf | HaSiS | CSD; **not** a Pedra dep |
| 74 | R074 | 2011 | ICDE | htap | C | listed | HyPer | fork snapshot |
| 75 | R075 | 2022 | SIGMOD | htap | R | listed | Diva | version chains kill scans |
| 76 | R076 | 2017 | SIGMOD | htap | C | listed | HTAP tutorial (Özcan) | historical baseline |
| 77 | R077 | 2022 | SIGMOD | htap | R | listed | Proteus | adaptive layout |
| 78 | R078 | 2012 | IEEE DEB | htap | C | listed | SAP HANA overview | delta + main |
| 79 | R079 | 2010 | VLDB | htap | C | listed | HYRISE | column groups |
| 80 | R080 | 2016 | SIGMOD | htap | C | listed | Bridging the Archipelago | TILE |
| 81 | R081 | 2014 | SIGMOD | htap | C | listed | H2O | adaptive store |
| 82 | R082 | 1992 | TODS | tx | C | listed | ARIES | WAL vocabulary |
| 83 | R083 | 1995 | SIGMOD | tx | C | listed | Critique of ANSI SQL isolation | name the phenomena |
| 84 | R084 | 2008 | SIGMOD | tx | C | listed | Serializable isolation for SI | SSI original |
| 85 | R085 | 2012 | VLDB | tx | C | listed | SSI in PostgreSQL | SSI as shipped |
| 86 | R086 | 1981 | TODS | tx | C | listed | Optimistic concurrency control | `tx.rs` |
| 87 | R087 | 2009 | NSDI | test | C | listed | MODIST | DST ancestor |
| 88 | R088 | 2014 | OSDI | test | C | listed | All File Systems Are Not Created Equal | fsync folklore is false |
| 89 | R089 | 2018 | OSDI | test | C | listed | Protocol-Aware Recovery | Raft/Montanha logs |
| 90 | R090 | 2013 | SOSP | layer | C | listed | Naiad | incremental fold vocab |
| 91 | R091 | 2006 | OSDI | test | C | listed | EXPLODE | storage model checking |
| 92 | R092 | 2016 | EDBT | ds | C | listed | The RUM conjecture | read / update / memory |
| 93 | R093 | 2019 | CIDR | ds | C | listed | Design Continuums | knob continuum |
| 94 | R094 | 1990 | TOPLAS | tx | C | listed | Linearizability | what `get_strong` means |
| 95 | R095 | 2007 | PODC | dist | C | listed | Paxos Made Live | consensus as engineering |
| 96 | R096 | 1984 | TOCS | test | C | listed | End-to-End Arguments | cursor-after-apply |
| 97 | R097 | 2013 | ICDE | ds | C | listed | Adaptive Radix Tree | MemTable alt; keep BTreeMap |
| 98 | R098 | 2018 | SIGMOD | ds | C | listed | The Case for Learned Indexes | pair with Bourbon |
| 99 | R099 | 1981 | CSUR | tx | C | listed | Concurrency Control in DDBMS | 2PL/OCC map |
| 100 | R100 | 2022 | SIGMOD | engine | R | listed | Dissecting LSM-based data stores | tutorial on R012 |

\*TurtleKV: accepted VLDB 2026 (lab page, 2026-01). Treat as preprint until the camera-ready is filed.

## By layer (counts)

| Layer | n | What we steal |
|-------|--:|---------------|
| `engine` | 39 | compact, stall, vlog, SST, Rocks/Pebble/Splinter |
| `htap` | 17 | triangle, learner, **refuse** hardware deps |
| `dist` | 12 | FDB roles, Raft, geo, leases |
| `layer` | 10 | Record Layer, SQL-on-KV, fold, Aurora log |
| `ds` | 9 | filters, RUM, ART, learned indexes |
| `tx` | 7 | ARIES, isolation, OCC, linearizability |
| `test` | 6 | DST, crash, YCSB, end-to-end |

## Local PDFs already in the repo

Do not re-fetch. Fichar from these first (Wave 0 in `queue.md`):

| ID | Path |
|----|------|
| R005 | `docs/references/wisckey-fast2016.pdf` |
| R006 | `docs/references/monkey-sigmod2017.pdf` |
| R007 | `docs/references/dostoevsky-sigmod2018.pdf` |
| R032 | `docs/references/realtime-lsm-htap-2022.pdf` |
| R041 | `docs/references/percolator-osdi2010.pdf` |
| R048 | `docs/references/tidb-raft-htap-vldb2020.pdf` |
| R066 | `docs/references/htap-databases-survey-arxiv2404.15670.pdf` |
| R068 | `docs/references/bytehtap-vldb2022.pdf` |
| R069 | `docs/references/vedb-htap-vldb2025.pdf` |
| R070 | `docs/references/polardb-imci-sigmod2023.pdf` |
| R071 | `docs/references/polynesia-icde2022.pdf` |
| R072 | `docs/references/pushtap-asplos2025.pdf` |
| R073 | `docs/references/hasis-fast2025.pdf` |

Open URLs for the rest live in `catalog.tsv` column `pdf_url` (empty means "find the official PDF when we fetch"; do not guess a paywall HTML into `pdfs/`).

## What this list is for, next

1. Pop [`queue.md`](queue.md). Fetch missing PDFs with `scripts/fetch-one.sh`.
2. Fichar into `notes/Rxxx-….md`.
3. Move one line into [`correlacao/01_paper_para_pedra.md`](correlacao/01_paper_para_pedra.md).
4. After ~20 fichas, demote / promote against [`BACKLOG.md`](BACKLOG.md).

Candidates that lost the cut (FPGA compact, Optane, SlateDB-the-repo, Tidehunter, …) are in the backlog on purpose.
