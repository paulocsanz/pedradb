# Correlation — paper → Pedra / Montanha

**Updated:** 2026-08-14
**Status:** correlação; bloco “settled” = PDF no disco, **não** ficha D3
ainda. Norma: [`../QUALIDADE.md`](../QUALIDADE.md).

**Rule:** add a row only when the paper is `have-pdf` or `ficha`. Rows below that are only `listed` are *intended* mappings (hypotheses), grouped so the next reader knows where to look. Promote a row out of "intended" when the ficha lands.

Existing product analysis (not replaced by this file):

- [`docs/engine-landscape-and-ideal-path.md`](../docs/engine-landscape-and-ideal-path.md)
- [`docs/rocksdb-critiques-and-improvements.md`](../docs/rocksdb-critiques-and-improvements.md)
- [`docs/rfc/0012-research-decisions.md`](../docs/rfc/0012-research-decisions.md)
- [`docs/htap-storage-primitives-and-research.md`](../docs/htap-storage-primitives-and-research.md)
- [`docs/rfc/0024-montanha-fold-for-caixote.md`](../docs/rfc/0024-montanha-fold-for-caixote.md)

## Settled mappings (PDF on disk in this repo)

| ID | Paper | Pedra target | Current decision |
|----|-------|--------------|------------------|
| R005 | WiscKey FAST'16 | `pedradb-core` vlog | **ficha D4.** Spill `SHIP` (L4). Rewrite GC `SHIP` (L5a). Incremental head/tail `OPEN` (L5). Drop WAL `REFUSE` (L5b). |
| R006 | Monkey SIGMOD'17 | SST bloom FPR allocation | uniform Bloom shipped; Monkey allocation **not** shipped |
| R007 | Dostoevsky SIGMOD'18 | compact policy | Lazy Leveling **do not ship** until measured |
| R032 | Real-Time LSM / LASER | HTAP layout-in-LSM | projection / learner only; not a second primary |
| R041 | Percolator OSDI'10 | Montanha TX | 2PC/OCC face; fichar against store 2PC |
| R048 | TiDB VLDB'20 | Montanha + fold/HTAP | learner replica = fold/column path, not voter |
| R066 | HTAP survey 2024 | product shape | triangle is doctrine; see HTAP note §0 |
| R068 | ByteHTAP VLDB'22 | layer 3 | composed vs converged: we pick named freshness |
| R069 | veDB-HTAP VLDB'25 | layer 3 | same |
| R070 | PolarDB-IMCI SIGMOD'23 | layer 3 | intelligent routing ≈ planner, not kernel |
| R071 | Polynesia ICDE'22 | hardware HTAP | do not depend on PIM |
| R072 | PUSHtap ASPLOS'25 | hardware HTAP | do not depend on CXL/PIM for P0 |
| R073 | HaSiS FAST'25 | CSD single-index | do not depend on CSD |

## Intended mappings (not fichado)

Use this as the reading order inside each crate. Details and URLs live in `CATALOG.md`.

### `pedradb-core` — WAL, memtable, SST, compact, bloom, vlog, TX

| IDs | Why |
|-----|-----|
| R001 O'Neil LSM, R004 bLSM, R009–R012 Rocks experience + compaction space | vocabulary and the incumbent we are measured against |
| R013 Spooky, R014–R015 Endure, R016 ADOC, R017 SILK | next compact / stall work after RFC-0014 |
| R018 HashKV, R031 Lethe | vlog GC + deletes that must actually disappear |
| R022–R024 REMIX / Disco / GRF | range amp when `scan` is real |
| R027–R028 EcoTune / How-to-grow | whether we ever add per-level T |
| R029–R030 SplinterDB | NVMe-era alternative; steal ideas, not the Bε-tree, unless measured |
| R033 SuRF, R019 Bourbon | filters / learned — likely refuse |
| R082 ARIES, R088 Pillai | recovery + fail-closed I/O |
| R086 OCC, R083 isolation critique, R085 SSI Postgres | TX isolation names we already expose |

### `pedradb-sim` / `pedradb-dst`

| IDs | Why |
|-----|-----|
| R043 FDB, R087 MODIST | DST as a development method, not a tool add-on |
| R088 Pillai, R089 Alagappan | torn write + consensus recovery |
| R010 RocksDB experience §testing | what production LSM teams actually test |

### Montanha / `pedradb-store` / `pedradb-raft`

| IDs | Why |
|-----|-----|
| R042 Spanner, R043 FDB, R045–R047 Cockroach 2020/22/25, R056 leases 2026 | unbundled TX, multi-region, leases |
| R041 Percolator | OCC + oracle + 2PC on KV |
| R049–R050 OceanBase | LSM in a distributed RDBMS; intra-L0 compact |
| R052 Raft, R053 Calvin | consensus vs deterministic apply |
| R059 CaaS-LSM, R060 SpanDB | disagg compact / hybrid device — P2 |

### Layers (`-fold`, `-sql`, `-dcs`, `-stream`, `-replicate`)

| IDs | Why |
|-----|-----|
| R044 Record Layer | indexes + records as KV projections |
| R048 TiDB / TiFlash | learner materializer |
| R090 Naiad | incremental fold vocabulary |
| R054 Aurora, R055 Snowflake | log + layers / cloud disagg (product, not kernel) |
| R066–R081 HTAP set | planner + projection, not SST flags |

## Conflicts to resolve with numbers (do not pick a side in prose)

| Tension | Papers | What closes it |
|---------|--------|----------------|
| Monkey FPR vs uniform Bloom | R006 vs shipped RFC-0014 | bench + a DST that the allocator cannot lie about miss rate |
| Lazy Leveling vs current whole-merge | R007 vs RFC-0012 | `baseline` write amp + space amp on one named workload |
| Vlog GC algorithm | R005, R018 | a GC that never resurrects, measured on large-value soak |
| LSM vs Splinter/Turtle hybrid | R029, R040 | only if we lose a published workload by a lot |
| Compact-as-a-service | R059 | only after local compact is boring and correct |
| Column-in-kernel vs learner | R032 vs R048 vs HTAP note | product RFC, not a compact patch |

## How to add a row

After a ficha: one line in the "Settled mappings" table, move the id out of "Intended", and if a tension is closed, replace the tension row with the decision + date.
