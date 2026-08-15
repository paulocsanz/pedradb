# Backlog — candidates not in the 100

**Updated:** 2026-08-14

These are real papers (or strong engineering artifacts). They lost the 100 because they are hardware-special, too far from a Pedra primitive, or we already have a better representative. Promote only via [`PROCESS.md`](PROCESS.md) re-rank.

## Recent LSM / engine (2023–2026)

| Paper | Year | Why not in the 100 | Promote if |
|-------|------|--------------------|------------|
| Xanthakis et al. *vLSM* arXiv:2407.15581 | 2024 | preprint; tail-latency claim overlaps SILK/ADOC | venue confirmed + fichado SILK |
| Bortnikov et al. *KV-Tandem* arXiv:2411.11091 | 2024 | tandem / pinned index; need camera-ready | we start pinning filter+index |
| Liu et al. *ArceKV* arXiv:2508.03565 | 2025 | workload-driven compact; overlaps DOPA/Endure | after Endure ficha |
| Chursin et al. *Tidehunter* arXiv:2602.01873 | 2026 | large-value store; overlaps WiscKey/HashKV | vlog GC design starts |
| *MountDB / learned indexes in RocksDB* arXiv:2605.23815 | 2026 | learned SST index; we refuse until Bourbon is fichado and refused/accepted | Bourbon ficha says "yes" |
| *FlintKV* arXiv:2607.02401 | 2026 | "fast durable engine"; verify venue | independent of Rocks, measured |
| *Resystance* (eBPF compaction) arXiv:2603.05162 | 2026 | Linux-specific; not a Pedra portable primitive | we adopt io_uring *and* this still matters |
| Keigo (co-design LSM + kernel) arXiv:2506.14630 | 2025 | kernel co-design | io_uring crate is the product path |
| STEM (FPGA compact) ICDE'24 | 2024 | FPGA | never, unless we sell an appliance |
| gLSM (GPGPU compact) ToS'24 | 2024 | GPU | never for P0 |
| D2Comp (DPU compact) TACO'24 | 2024 | DPU | disagg product exists |
| Prism / PrismDB ASPLOS'23 | 2023 | Optane / XPoint — device we will not assume | device reappears in commodity form |
| MatrixKV ATC'20, TriangleKV TPDS'22 | 2020–22 | NVM container | same |
| ELECT FAST'24 | 2024 | erasure-coding tiering | we have multi-AZ object tier |
| ELMo-Tune HotStorage'24 / V2 2025 | 2024–25 | LLM tuner; survey itself measured ~100s | we want offline recs only |
| DOPA-DB HotStorage'24 | 2024 | dynamic compact size | after design-space ficha |
| Calcspar ATC'23 | 2023 | cloud latency contracts | object-store backend |
| MirrorKV SIGMOD'23 | 2023 | hybrid cloud | object-store backend |
| CaaS-LSM SIGMOD'24 | 2024 | compact as a service | local compact is boring |
| Hailstorm ASPLOS'20 | 2020 | disagg LSM | Montanha disagg |
| SA-LSM VLDB'22 | 2022 | survival analysis layout | we have cold-tier |
| Leaper VLDB'20, AC-Key ATC'20 | 2020 | cache / prefetch | cache is a problem |
| Vigil-KV ATC'22 | 2022 | NVMe determinism | io_uring path |
| SpanDB FAST'21 | 2021 | WAL on fast SSD, data on cheap | two-device deploy |
| DEPART FAST'22 | 2022 | replica decoupling | geo |
| LSM-VEC 2025 | 2025 | vector index on LSM | we sell vectors |
| AdCache EDBT'26 | 2026 | RL cache | cache is a problem |

## Data structures

| Paper | Why wait |
|-------|----------|
| Leis et al. ART (ICDE'13) + ART That Lasts (SIGMOD'26) | MemTable is BTreeMap on purpose; reopen with a CPU-bound write bench |
| Kraska et al. The Case for Learned Index Structures (SIGMOD'18) | Bourbon is the LSM-shaped version; start there |
| Ferragina / PGM index | same |
| Breslow Morton filters; quotient filters | after Ribbon/Chucky fichas |
| Graefe B-tree surveys | if we ever grow a Redwood-class engine |
| PebblesDB SOSP'17 (fragmented LSM / guards) | *is* in the 100 as R008 — listed here only if demoted |

## Distributed / TX / layers

| Paper | Why wait |
|-------|----------|
| CRDB *Scalable Leader Leases* SIGMOD'26 | in the 100 as R056; fetch when Multi-Raft is the work |
| Yugabyte / PolarDB-X / GaussDB industry papers | one representative per architecture is enough (OceanBase + CRDB + TiDB + FDB) |
| Neon / Multigres / Aurora Limitless | serverless Postgres face — product, after Aurora ficha |
| Snowflake Unistore / Hybrid tables | HTAP composed; Vanlightly essays already tracked in HTAP note |
| etcd raft thesis / Ongaro dissertation | Raft paper first |
| Flexible Paxos, EPaxos, Raft does not guarantee liveness in practice | when we change the consensus module |
| Jepsen reports (etcd, TiKV, NATS, Mongo, ES) | not papers; link from DST notes when we fichar a system |

## Fold / incremental / streams

| Paper | Why wait |
|-------|----------|
| DBToaster (incremental view maintenance) | after Naiad ficha |
| Differential Dataflow / Timely | after Naiad; do not import the runtime |
| Materialize / Feldera industry | same |
| Flink + Paimon / Fluss LTAP | lakehouse; HTAP note already names this as composition |
| Saltzer/Reed/Clark End-to-End Arguments (1984) | cite from fold fichas; not a DB paper |

## Engineering artifacts (not in the 100 on purpose)

Keep using them; they are not "papers":

- Pebble announcement + `pebble/docs/rocksdb.md` → `docs/references/`
- fjall 2.0 announcement
- SlateDB design docs (object-store LSM; DST in Rust)
- Badger / Titan / BlobDB design notes
- Slipstream / Quicksilver sources (already snapshotted)
- FDB `testing.html`, Redwood docs (no Redwood *paper*)
- Will Wilson Strange Loop 2014 (DST talk)
- Antithesis / TigerBeetle / WarpStream DST writeups
- Jack Vanlightly HTAS / taxonomy essays (HTAP note §11.2)

## Demoted from the 100

_None yet. When a paper leaves `CATALOG.md`, land here with date + reason._
