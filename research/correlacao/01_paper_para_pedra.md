# Correlation — paper → Pedra / Montanha

**Updated:** 2026-08-15
**Status:** correlação. Settled **com ficha:** R005–R007, R010, R012–R014, R016–R018, R031, R041, R043–R045, R048. O resto da
tabela “settled mappings” ainda é PDF-no-disco sem D3. Norma:
[`../QUALIDADE.md`](../QUALIDADE.md).

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
| R006 | Monkey SIGMOD'17 | SST bloom FPR allocation | **ficha D4.** L1 Bloom-por-SST `SHIP`. L2 FPR \(\propto n_i\) `MEASURE` (HDD 80% ≠ Pedra; \(L\le 4\)) |
| R007 | Dostoevsky SIGMOD'18 | compact policy | **ficha D4.** L3 Lazy Leveling `REFUSE` (exige L2; short range piora; \(L\le 4\)) |
| R010 | Rocks Experience FAST'21 | kernel / ops | **ficha D4.** L9 `SHIP`. Espaço>WA. L25–L27 `MEASURE` |
| R018 | HashKV ATC'18 | `vlog` / 0028 | **ficha D4.** L19 hash-groups `MEASURE` (não P0). Fig. 2 19.7× mata tail como próximo GC. L4/L5b confirmados |
| R012 | Compaction design space VLDB'21 | `compact_levels` | **ficha D4.** 4 primitivas. L35 LO+1 `MEASURE`. L36 menu `REFUSE`. L37 universal `REFUSE`. L38 agora na ficha R031 |
| R031 | Lethe SIGMOD'20 | tombstone / compact | **ficha D4.** L38 FADE `MEASURE` (depois L35, com SLA). L43 KiWi `REFUSE` |
| R013 | Spooky VLDB'22 | `compact_levels` | **ficha D4.** L11 Spooky `MEASURE` depois de L35. Full-do-par ≠ Full-de-\(L\) (50%) |
| R014 | Endure VLDB'22 | knobs / RFC-0012 | **ficha D4.** L12 robust static `MEASURE` (0 knobs hoje). Sempre leveling. Sem tuner |
| R016 | ADOC FAST'23 | flush / stall | **ficha D4.** L41 taxonomia MMO/L0O/RDO. L42 tuner `REFUSE`. Complementar a SILK |
| R017 | SILK ATC'19 | flush / p99 | **ficha D4.** L41 stall names + flush>L0 `MEASURE`. Não scheduler completo |
| R032 | Real-Time LSM / LASER | HTAP layout-in-LSM | projection / learner only; not a second primary |
| R041 | Percolator OSDI'10 | Montanha TX | **ficha D4.** L22/L23 `REFUSE`. OCC kernel > SI. Store 2PC ≠ Figs. 4–6 |
| R043 | FoundationDB SIGMOD'21 | Montanha + DST | **ficha D4.** L9 layers. L28 swarm MEASURE. L29 role-split `REFUSE`. L30 5 s `REFUSE` |
| R044 | Record Layer SIGMOD'19 | `-index` / `-sql` / recipes | **ficha D4.** L9 layers. L31 same-TX idx `SHIP`. L32 produto RL `REFUSE`. L33 VERSION `MEASURE`. L34 atomics `REFUSE` |
| R045 | CockroachDB SIGMOD'20 | Montanha store / `-raft` | **ficha D4.** L9/L29 confirmados. L39 lease `MEASURE`. L40 HLC/intents CRDB `REFUSE` |
| R048 | TiDB VLDB'20 | Montanha + fold/HTAP | **ficha D4.** L8 fold≠voter `SHIP`. L24 read-index no fold `REFUSE`. TiFlash ≠ fold |
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
| R001 O'Neil LSM, R004 bLSM, R009 Rocks workloads | vocabulary and the incumbent we are measured against |
| R022–R024 REMIX / Disco / GRF | range amp when `scan` is real |
| R027–R028 EcoTune / How-to-grow | whether we ever add per-level T |
| R029–R030 SplinterDB | NVMe-era alternative; steal ideas, not the Bε-tree, unless measured |
| R033 SuRF, R019 Bourbon | filters / learned — likely refuse |
| R082 ARIES, R088 Pillai | recovery + fail-closed I/O |
| R086 OCC, R083 isolation critique, R085 SSI Postgres | TX isolation names we already expose |

### `pedradb-sim` / `pedradb-dst`

| IDs | Why |
|-----|-----|
| R043 FDB, R087 MODIST | **R043 ficha D4.** DST = método (sim do binário + seed). L28 MEASURE swarm |
| R088 Pillai, R089 Alagappan | torn write + consensus recovery |
| R010 Rocks Experience FAST'21 | **ficha D4.** L9 confirmado. Espaço>WA. L25 WAL-skip MEASURE; L26 file checksum; L27 user-ts |

### Montanha / `pedradb-store` / `pedradb-raft`

| IDs | Why |
|-----|-----|
| R042 Spanner, R046–R047 Cockroach 2022/25, R056 leases 2026 | multi-region / serverless / leases — depois de L39 |
| R041 Percolator | OCC + oracle + 2PC on KV |
| R049–R050 OceanBase | LSM in a distributed RDBMS; intra-L0 compact |
| R052 Raft, R053 Calvin | consensus vs deterministic apply |
| R059 CaaS-LSM, R060 SpanDB | disagg compact / hybrid device — P2 |

### Layers (`-fold`, `-sql`, `-dcs`, `-stream`, `-replicate`)

| IDs | Why |
|-----|-----|
| R044 Record Layer | **ficha D4.** indexes + records as KV projections in the same TX (L31). Not the Java RL product (L32) |
| R048 TiDB / TiFlash | learner **papel** (não voter); materializer colunar + read-index **não** são o fold |
| R090 Naiad | incremental fold vocabulary |
| R054 Aurora, R055 Snowflake | log + layers / cloud disagg (product, not kernel) |
| R066–R081 HTAP set | planner + projection, not SST flags |

## Conflicts to resolve with numbers (do not pick a side in prose)

| Tension | Papers | What closes it |
|---------|--------|----------------|
| Monkey FPR vs uniform Bloom | R006 vs shipped RFC-0014 | bench + a DST that the allocator cannot lie about miss rate |
| Lazy Leveling vs current whole-merge | R007 vs RFC-0012 | **fechado L3 REFUSE** (ficha R007). Reabrir só com o gate da ficha |
| Full-do-par vs file-granular LO+1 | R012 vs `compact_levels` | L35: WA/stall no `benches/baseline` com \(L=3\) |
| Vlog GC algorithm | R005, R018 | **R018 D4:** tail = Fig. 2 19.7×. Hash groups L19 MEASURE. Rewrite/blobs ficam até soak nomeado perder |
| LSM vs Splinter/Turtle hybrid | R029, R040 | only if we lose a published workload by a lot |
| Compact-as-a-service | R059 | only after local compact is boring and correct |
| Column-in-kernel vs learner | R032 vs R048 vs HTAP note | product RFC, not a compact patch |

## How to add a row

After a ficha: one line in the "Settled mappings" table, move the id out of "Intended", and if a tension is closed, replace the tension row with the decision + date.
