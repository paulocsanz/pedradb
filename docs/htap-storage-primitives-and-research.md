# HTAP: storage primitives, research frontier, and PedraDB

**Status:** research note (not an RFC, not a product commitment) — **canonical** for HTAP physics + literature  
**Updated:** 2026-08-12  
**Scope:** what HTAP actually is, the storage primitives that make a *good* one, primary literature (surveys, systems papers, hardware papers, dissertations), and what that implies for PedraDB / Montanha.

**Product ambition (one node primitive → multi-leader face):**  
[`node-primitive-and-unified-platform.md`](node-primitive-and-unified-platform.md) — *how* we package SoR + projections + N leaders. **This file** is *why* the triangle forces that packaging.

**Primary PDFs:** [`docs/references/`](references/) — see [§11](#11-bibliography-local-copies). Read papers, not abstracts.

This note is written from primary sources in §11. Industry blogs are labeled as such. A vendor claim is not treated as a theorem.

---

## 0. Executive lock-in (for Pedra / Montanha)

HTAP is **not** a magic motor. It is a **triangle** — **layout right for each load**, **data freshness**, **performance isolation** — and **no pure-software system sits on all three vertices at once**. “Excellent” means **choosing the point on purpose** and making that point **cheap with the right primitive**.

| Settled fact | Implication for us |
|--------------|-------------------|
| Row for point; column for scan | Pedra LSM = OLTP path; column = **projection / learner**, not “flag on SST” |
| Freshness ↔ isolation trade | Named contract: e.g. `olap=Applied` lag p99; not “real-time” |
| Dual-format has write amp | Budget it; selective materialization; no second fsync on OLTP commit |

**Three honest product shapes** (detail §10.3): (1) **composed** CDC → CH/DuckDB/Iceberg; (2) **TiFlash-shaped** Raft learner → columnar; (3) **LASER-in-kernel** only if proven. (2)+(3) can compose (LASER as learner local format).

**Generic HTAP engine for “all workloads”** = **one SoR log+TX+keys** + **planners that route** + **projections** — not one query engine good at everything, not dual primary in one MANIFEST.

**Product vignette (target UX):** Postgres-shaped **write-optimized primaries**, **N primaries in M regions** (range leaders), **RO replicas** (promote on failover), **OLAP RO replicas** for logs/telemetry/scan — **one place**, no separate ClickHouse install; the catalog/planner **reads the right path**. Full write-up: [node-primitive §6](node-primitive-and-unified-platform.md#6-product-face-postgres-for-product-tables--clickhouse-for-logs--automatic).

---

## 1. What is actually settled

HTAP (Hybrid Transactional/Analytical Processing) is Gartner's 2014 name for running OLTP and OLAP on the same data without a nightly ETL. The *need* is real: fraud, pricing, ads, inventory, "what just happened". The *architecture* is not one thing.

Three facts survive every survey and every 2024–2026 system paper:

1. **Row layout is right for point reads/writes; column layout is right for scans/aggregates.** This is not a fashion. It is I/O physics (NSM vs DSM; Abadi/Boncz/Idreos column-store survey; PAX as the intra-page compromise).
2. **Freshness and performance isolation trade off.** Same memory/CPU/I/O → high freshness, high interference. Separate replica/format → isolation, stale analytics until merge. Zhang, Li et al. make this the central axis of their 2024 survey. No production system has deleted the tradeoff; they pick a point on it.
3. **Write amplification + format conversion is the tax of dual representation.** Dual-format (row + column) at least doubles write I/O and adds CPU for conversion. Immediate sync keeps analytics fresh and hurts OLTP; delayed sync protects OLTP and makes analytics stale. The 2024 survey and later industry writeups treat this as a *constraint*, not a missing trick.

"Excellent HTAP" therefore cannot mean "one engine, both workloads at 100%, instant freshness, zero extra I/O". That triple is the design *goal* stated by Polynesia and PUSHtap; both papers then introduce new hardware to *approximate* it. In software-only systems, excellence means: pick the point on the triangle deliberately, make that point cheap with the right primitive, and expose a *named* freshness contract.

---

## 2. What HTAP is not

| Term | Meaning | Not the same as |
|------|---------|-----------------|
| **HTAP** | One *system* serves both workloads; specialized storage + query paths stitch row and column. Visibility to analytics is part of the system. | Two databases + nightly ETL |
| **HTAS** (Vanlightly 2024) | One *storage engine* serves transactional and analytical *access* (often stream + Iceberg), without claiming one query engine. Consistency is looser by design. | SAP HANA / Hyper |
| **LTAP / shared tiering** | Two systems, two workloads, one durable copy of the *cold* tier (e.g. Fluss → Iceberg; Databricks LTAP). Hottest data lives only in system A. | Instant visibility |
| **Zero-ETL / composed** | Two engines + managed CDC (Aurora→Redshift, Postgres→ClickHouse). Marketing name for a pipeline. | Single-engine HTAP |
| **Lakehouse** | Analytical table format (Iceberg/Delta/Hudi) with ACID *for files*. Not OLTP. | Row-store |

ClickHouse's March 2026 engineering essay argues the industry answered "composed, not converged." That is a *market observation* (Snowflake bought Crunchy Data / Postgres; Databricks bought Neon; Microsoft deprecated Synapse Link for Fabric Mirroring; AWS ships Aurora zero-ETL). It is not a proof that single-system HTAP is impossible. PolarDB-IMCI, veDB-HTAP, TiDB+TiFlash, and SingleStore are still sold as HTAP and still run at scale. What *is* established: at warehouse scale, the winning *product* shape in 2025–2026 is often two engines plus a sync layer.

---

## 3. The triangle (and how systems pick a vertex)

An ideal HTAP wants three properties (Polynesia ICDE'22, PUSHtap ASPLOS'25, Zhang/Li survey):

| Property | Meaning |
|----------|---------|
| **Workload-specific layout** | OLTP on rows / indexes; OLAP on columns / vectors |
| **Freshness** | Analytics sees recent commits (µs … seconds, *named*) |
| **Isolation** | Mixed load does not collapse OLTP p99 |

Software systems pick two and degrade the third:

| Architecture (Zhang/Li 2024) | Freshness | Isolation | Scale | Example |
|------------------------------|-----------|-----------|-------|---------|
| (a) Primary row + in-memory column | High | Low | Low | Oracle IMCS, SQL Server CSI, DB2 BLU |
| (b) Distributed row + column replica | Low–medium | High | High | TiDB/TiFlash, F1 Lightning |
| (c) Primary row + distributed IMCS | Medium | High | OLAP high, OLTP low | MySQL HeatWave |
| (d) Primary column + delta row | High | Low | OLTP low | SAP HANA, Hyper |

Vanlightly (June 2026) refines hybrid into two *visibility* mechanisms:

- **Freshness-by-composition:** write both formats, or merge-on-read so an OLAP query is consistent without waiting. Examples: SingleStore, HANA, Snowflake Hybrid tables.
- **Freshness-by-catchup:** OLAP waits until the column store has applied up to the query LSN. Examples: PolarDB-IMCI intelligent routing, TiFlash.

HaSiS (FAST'25) compresses the same space into three *index* categories:

| Design | Copies | Storage | Claimed freshness |
|--------|--------|---------|-------------------|
| Multi-index, multi-store | 2 | Disk | 20 ms – 8 min |
| Multi-index, single-store | 1 | Memory | Instant |
| Single-index, single-store (HaSiS) | 1 | Disk (CSD) | Instant |

Those numbers are *from HaSiS's table*, citing the source systems; they are not an independent re-measurement.

---

## 4. Storage primitives (the actual building blocks)

These are the pieces you compose. An "excellent" HTAP is a *selection* of these, not a new SQL dialect.

### 4.1 Layout primitives

| Primitive | What it is | Cost it pays | Where it lives |
|-----------|------------|--------------|----------------|
| **NSM (row / N-ary storage model)** | Whole tuple contiguous | Scans read unused columns | Every OLTP engine |
| **DSM (column / decomposition)** | One column contiguous | Point lookup is N I/Os; updates rewrite columns | ClickHouse, Parquet, CSI |
| **PAX / hybrid page** | Row-group, columns inside the page | Compromise; page size becomes a first-class knob | Hyrise chunks, HaSiS hybrid page, PolarDB-IMCI column index |
| **Column groups (CG)** | Co-accessed columns stored together | Design space exponential in #columns | HYRISE, H2O, TILE, LASER, Proteus |
| **Lifecycle layout** | *Different* layout as data ages | Compaction must *reshape*, not only merge | Real-Time LSM / LASER |
| **Simulated columns** | Key stored with each CG (LSM-friendly) | Extra key bytes; compression recovers most | LASER on RocksDB |

### 4.2 Version / snapshot primitives

| Primitive | What it is | HTAP role |
|-----------|------------|-----------|
| **MVCC version chain** | OLAP walks visible versions | High freshness; long chains kill scans (Weaver, Diva) |
| **Copy-on-write / fork snapshot** | Hyper: `fork` the OLTP process, OLAP on snapshot | µs freshness; RAM and isolation suffer |
| **Delta + main** | HANA L1-row → L2-column → Main; SQL Server tail index | Instant insert, merge cost later |
| **Per-page clustered versions** | All versions of a page stay together | OLAP does not chase a heap of versions (HaSiS) |
| **Global timestamp / LSN** | One number both engines can pin | Strong snapshot across row and column (ByteHTAP, PolarDB-IMCI, TiDB) |

### 4.3 Synchronization primitives

| Primitive | Mechanism | Freshness | Isolation |
|-----------|-----------|-----------|-----------|
| **In-memory delta merge** | Threshold / delete-table / dictionary | High | Low (same box) |
| **Log shipping + replay** | WAL / Raft log / redo → column store | Medium (ms–s) | High |
| **Physical redo replay** | Apply *physical* page/redo to a different layout | PolarDB-IMCI: <30 ms typical, <5% OLTP hit | High (RO nodes) |
| **Logical CDC / binlog** | Row changes as events | Seconds (ClickPipes ~10 s) | Highest |
| **Raft learner** | Non-voting replica transforms row→column | TiFlash: hundreds of ms typical | High |
| **Merge-on-read** | Query reads base columns + delta | Fresh at query time | Pays scan tax |
| **Merge-on-write / catch-up** | Query waits for apply to LSN | Fresh when served | Pays wait |
| **Commit-ahead log shipping (CALS)** | Ship redo *before* commit returns | PolarDB-IMCI freshness trick | Must not add fsync to OLTP path |

### 4.4 Isolation primitives

| Primitive | What it is |
|-----------|------------|
| **Resource partition** | Cores/memory reserved per workload (HANA). Wastes idle capacity. |
| **Replica isolation** | OLAP on another node/replica (TiFlash, PolarDB RO, BatchDB). |
| **Scheduler isolation** | OLTP priority, OLAP throttle. Protects p99, wrecks analytics SLOs. |
| **Compute/storage disaggregation** | Scale OLAP compute without touching the writer (PolarDB, ByteHTAP, AlloyDB). |
| **Hardware islands** | Polynesia: TP island (CPU+cache) vs AP island (PIM). Isolation in silicon. |

### 4.5 Access / execution primitives (above storage, but they *drive* storage)

| Primitive | Why storage cares |
|-----------|-------------------|
| **Hybrid row/column scan** | Optimizer picks engine per operator (TiDB, SQL Server, veDB router) |
| **Late materialization + SIMD** | Column pages must be aligned and compressible |
| **Zone maps / min-max / bloom** | Data skipping is the OLAP "index" |
| **Delete bitmaps** | ByteHTAP: cheap deletes without rewriting columns |
| **RID / PK locator** | PolarDB-IMCI: two-layer LSM maps PK → row-id so column index can be insert-order |
| **WAL as product** | If the log is a first-class stream, CDC/TiFlash/HTAS become layers, not forks |

### 4.6 Hardware primitives (bleeding edge, 2022–2025)

| Primitive | Paper | What it buys |
|-----------|-------|--------------|
| **CSD transparent compression** | HaSiS FAST'25 | Sparse 128 KB PAX pages + per-page delta *without* wasting NAND. Decouples page size from write amp. |
| **PIM islands** | Polynesia ICDE'22 | OLAP next to DRAM; custom update-propagation hardware. Claimed 1.7×/3.7× TP/AP vs Hyper/AnkerDB/BatchDB models, −48% energy. |
| **Unified PIM format** | PUSHtap ASPLOS'25 | One layout that CPU (row-ish) and PIM (column-ish) can both use. 3.4×/4.4× vs multi-instance PIM. |
| **GPU as OLAP engine** | Caldera CIDR'17, RateupDB VLDB'21 | CoW or primary-secondary; freshness usually loses. |

These are not PedraDB P0. They define the *research ceiling*: if you assume only commodity NVMe, you cannot copy HaSiS or PUSHtap.

---

## 5. Systems map (what production actually shipped)

### 5.1 In-memory / single box (2010–2018)

- **Hyper** (Kemper/Neumann ICDE'11): `fork` snapshots. Later grew a column main + MVCC delta.
- **SAP HANA**: L1 row-delta → L2 dict-compressed → Main column. The textbook delta+main.
- **HYRISE** (Grund et al. VLDB'10; Hyrise re-engineered EDBT'19): adaptive column groups; later PAX chunks.
- **H2O** (Alagiannis/Idreos/Ailamaki SIGMOD'14): hands-free adaptive layout.
- **TILE** (Arulraj/Pavlo/Menon SIGMOD'16): "bridging the archipelago" — hybrid tiles for mixed workloads.
- **Oracle IMCS / SQL Server CSI / DB2 BLU**: row is primary; selected columns in memory.

### 5.2 Distributed NewSQL HTAP (2019–2022)

- **TiDB + TiFlash** (Huang et al. VLDB'20): TiKV row regions + Raft *learner* that materializes columns. Cross-engine optimizer. Isolation by node; freshness = log apply lag.
- **F1 Lightning** (Yang et al.): Spanner + changepump + LSM of row deltas collapsing into columns. Query window ~10 h.
- **SingleStore** (ex-MemSQL): in-memory row + on-disk columnar LSM; universal table. Cloud-native table storage.
- **ByteHTAP** (Chen et al. VLDB'22): *separate engines, shared storage*. ByteNDB (Aurora-like, "log is the database") + Flink. Global committed LSN. In-memory column delta + durable base. Freshness **< 1 s**, configurable. Delete bitmaps. Explicitly rejects "one engine" as too expensive to build.

### 5.3 Cloud-native (2022–2025)

- **PolarDB-IMCI** (Wang et al. SIGMOD'23): five design goals they actually measured — transparent SQL, OLAP ≈ ClickHouse-class, **<5% OLTP perturbation**, visibility delay **<5 s typical / <30 ms** under load they report, scale-out in tens of seconds. Mechanism: column index on **RO nodes**, **physical REDO** replay (not extra binlog), commit-ahead shipping, 2-phase conflict-free apply, insert-order column chunks + RID LSM. First industrial claim of *physical* redo into a *heterogeneous* layout.
- **MySQL HeatWave / AlloyDB columnar**: IMCS next to a PG/MySQL primary. AlloyDB columnar is RAM-bounded (Google's own limit).
- **Snowflake Unistore / Hybrid tables**: GA late 2024; ClickHouse (2026) reports ~1k ops/s and 500 GB caps — treat as a vendor critique, not a paper number. Snowflake then bought Crunchy Data (Postgres) rather than doubling down.
- **veDB-HTAP** (Chen et al. VLDB'25): successor to ByteHTAP. MySQL **Secondary Engine** (same seam as HeatWave), queries always enter MySQL, smart cost/ML router, proprietary MPP replaces Flink, unified RC isolation. Claimed **>3× TPC-H at 1/3 the resources** vs ByteHTAP. Adaptive execution + multi-tenant resource groups.

### 5.4 Composed / "HTAP is a pipeline" (2024–2026)

- **Aurora zero-ETL → Redshift**, **Fabric Mirroring**, **ClickPipes/PeerDB Postgres → ClickHouse**, **pg_clickhouse** FDW.
- **HTAS** (Vanlightly / Confluent): Kora as multi-modal storage — Kafka API + Freight (direct S3) + Tableflow (Iceberg). Not a SQL HTAP database.
- **Fluss / LTAP**: hot row/stream on specialized servers, cold shared Iceberg. One durable copy of history.

---

## 6. The storage papers that actually move the frontier

Read these, not the blog posts, if the question is "what would a *great* HTAP store look like".

### 6.1 Real-Time LSM-Trees / LASER — the PedraDB-shaped paper

Saxena, Golab, Idreos, Ilyas. arXiv:2101.06801 (v2 2022). Local copy: [`references/realtime-lsm-htap-2022.pdf`](references/realtime-lsm-htap-2022.pdf).

**Claim (from the paper, not a summary):** an LSM is already a lifecycle machine (new data at the top, old data at the bottom). Give *each level* its own column-group layout; reshape during compaction; pick layouts with a cost model.

Invariants they chose:

- Level-0 stays row-oriented (write path unchanged).
- **CG containment:** every CG at level *i* is a subset of one CG at *i−1* (makes reshape a split, not a join).
- No extra replica per level (OLTP stays on recent levels).
- Compaction is **CG-local** (a hot column does not drag a cold one down).

They implement **LASER** on RocksDB 5.14: simulated CGs (key+columns), partial-row updates, `LevelMergingIterator` + `ColumnMergingIterator`, cost-based design selection (Hyrise-style partition, per level). Design selection for 100 columns × 8 levels took **3 seconds** in their eval.

This is the only mature *kernel-level* HTAP idea that composes with a Rocks-class LSM without adding a second engine. It does **not** give you a SQL optimizer, SIMD runtime, or isolation. It gives PedraDB a *layout primitive* if we ever want one.

### 6.2 HaSiS — single index by cheating the SSD

Huang, Shen, Shao, Chen, Zhang. FAST'25. Local copy: [`references/hasis-fast2025.pdf`](references/hasis-fast2025.pdf).

**Claim:** multi-index HTAP is stuck because migrating across indexes costs freshness *or* I/O. Computational SSDs with transparent compression let you keep **sparse** 128 KB PAX pages + 16 KB row-delta pages; zeros compress away, so logical space ≠ NAND. One B+tree, instant freshness, page-clustered MVCC.

Depends on **ScaleFlux-class CSD**. Without that hardware, sparse large pages waste capacity and write amp returns. Prototype is open-sourced; compared to TiDB plus MySQL/PG/Parquet baselines.

### 6.3 PolarDB-IMCI — physical log into a foreign layout

Wang et al. SIGMOD'23. Local copy: [`references/polardb-imci-sigmod2023.pdf`](references/polardb-imci-sigmod2023.pdf).

The hard storage problem they solved: redo is *physical* (tied to row pages), column index is a different layout. They replay redo on the RO node by reconstructing logical ops against the row buffer pool, then applying out-of-place inserts to column chunks. That avoids a second logical log (no extra fsync on the writer). This is the cleanest published answer to "how does a cloud-native primary stay fast while replicas become columnar".

### 6.4 ByteHTAP → veDB-HTAP — shared log, two engines

ByteHTAP VLDB'22: [`references/bytehtap-vldb2022.pdf`](references/bytehtap-vldb2022.pdf).
veDB-HTAP VLDB'25: [`references/vedb-htap-vldb2025.pdf`](references/vedb-htap-vldb2025.pdf).

Storage lesson: if the primary already treats **log as the database** (Aurora/ByteNDB), the column store is another *subscriber* of that log. Delta (memory) + Base (column files) + global LSN is enough for <1 s SI. veDB then shows the *query* side was the remaining pain (Flink dialect, rule router, RC vs SI mismatch) — not the log primitive.

### 6.5 Hardware co-design — Polynesia and PUSHtap

- Polynesia ICDE'22: [`references/polynesia-icde2022.pdf`](references/polynesia-icde2022.pdf). They measure 43–75% TP loss and ~50% AP loss on snapshotting Hyper-style systems *because of data movement*, then invent islands + PIM to get it back. Open-source sim.
- PUSHtap ASPLOS'25: [`references/pushtap-asplos2025.pdf`](references/pushtap-asplos2025.pdf). Unified format so CPU and PIM do not fight. Single-instance HTAP on PIM.

These papers are evidence that **the software triangle is real enough that people are changing DRAM**. They are not a PedraDB implementation plan.

### 6.6 Adaptive layout / self-driving (still open)

- Proteus (Abebe et al. SIGMOD'22) + Tiresias (VLDB'22): learned placement of row vs column vs index.
- Peloton / NoisePage (Pavlo et al.): self-driving hybrid; Arrow-backed PAX.
- Mainlining (Li/Pavlo VLDB'20): transactional workloads *on* a universal columnar file (Arrow/Parquet). Opposite direction of LASER: start from columns, make OLTP survivable.

Zhang/Li list "data organization for *distributed* HTAP" as still open in 2024: which columns in RAM vs disk vs which node, which compression grain, online vs offline learning.

---

## 7. Dissertations and theses (the long-form work)

| Work | Where / year | Why it matters for HTAP storage |
|------|--------------|----------------------------------|
| **Martin Grund, HYRISE** | HPI / VLDB'10 paper is the thesis core | First serious adaptive column-group store. Every later CG paper cites it. |
| **Joy Arulraj, *The Design and Implementation of Non-volatile Memory Database Systems*** | CMU 2018, **SIGMOD Jim Gray Award 2019** | NVM recovery/storage; same author as TILE (SIGMOD'16) and Peloton hybrid layouts. NVM changes the "in-memory HTAP" assumption. |
| **Hemant Saxena, *Scalability aspects of data cleaning*** | Waterloo 2021 | Contains the Real-Time LSM / LASER line (with Idreos + Ilyas). The HTAP-storage dissertation closest to an LSM kernel. |
| **Chao Zhang, *Performance Benchmarking and Query Optimization for Multi-Model Databases*** | Helsinki 2021; later Tsinghua postdoc | Author of the 2024 HTAP survey and HyBench. Thesis is multi-model; the HTAP corpus is the postdoc work. |
| **Kecheng Huang** (CUHK) | FAST'22 passive persistence; FAST'25 HaSiS | Line of work: LSM logging → CSD-assisted single-index HTAP. Treat HaSiS as the dissertation-grade result even if the bound thesis is later. |
| **M. Kishore, *Can HTAP eliminate ETL? An empirical analysis*** | Purdue 2025 | Empirical, not a new engine. Useful as a *negative/conditional* result: when dual-engine HTAP does or does not replace ETL. Read before claiming "ETL is dead". |
| **Minxuan Zhou, PIM accelerators** | UCSD 2023 | Not HTAP-specific; background for Polynesia/PUSHtap. |

Also foundational, even if not titled "HTAP":

- Kemper/Neumann Hyper line (TUM) → Umbra.
- Viktor Leis LeanStore / SI-for-storage-engines (PVLDB'23) — snapshot isolation that does not fall over under long OLAP readers.
- Stratos Idreos: Monkey, Dostoevsky, Wacky continuum, H2O, LASER — the LSM *cost-model* school. PedraDB already cites Monkey/Dostoevsky; LASER is the HTAP chapter of that school.

---

## 8. Benchmarks (how "good" is measured)

If you cannot name the metric, you cannot claim excellence.

| Bench | What it adds |
|-------|----------------|
| **CH-benCHmark** (Funke et al.) | TPC-C + TPC-H on same schema. Classic, still used, not a real HTAP *app*. |
| **HTAPBench** | Isolation-aware mixed clients. |
| **OLxPBench** (Kang et al. ICDE'22) | Domain-specific, semantically consistent, real-time. |
| **HATtrick** | **Throughput frontier** (TP vs AP 2-D) + freshness score from a global clock. The right *picture* of the triangle. |
| **HyBench** (Zhang et al.) | FinTech schema, OLTP + OLAP + **OLXP** mixed ops, **H-Score** = geometric mean of TPS/QPS/XPS penalized by freshness. |
| **mOLxPBench / ADAPT / HAP** | Microbenches for *layout* (wide vs narrow tables, projection width). |

A PedraDB-layer HTAP that only quotes TPC-H is not an HTAP result. Quote a frontier + a freshness distribution.

---

## 9. What would make an *excellent* HTAP (operational definition)

Not a slogan. A checklist. Ranked.

**Must (or it is marketing):**

1. **Named freshness contract.** e.g. "SI at globally committed LSN, p99 visibility ≤ 200 ms" or "merge-on-read, always latest commit, scan tax bounded". Unnamed "real-time" is not a spec.
2. **Named isolation.** e.g. "OLTP p99 +<5% at a given AP QPS" (PolarDB-IMCI's bar) or "AP on separate RO/learner nodes".
3. **One durability story.** The OLTP commit path does not grow a second fsync for the column store (PolarDB's reason to reuse physical redo; ByteHTAP's reason to share the log).
4. **Layout that matches lifecycle.** Recent data row-friendly; cold data column-friendly *or* a single hybrid page that is not embarrassing at either. Dual full copies are allowed if you *budget* the write amp.
5. **A pin** (LSN / snapshot / timestamp) both engines understand. Without it you do not have a consistent HTAP read, you have two databases.

**Should (or it will not win against composed CDC):**

6. **WAL/redo as a product.** Learners, CDC, Iceberg materializers, and column indexes are all subscribers. If the log is not sequenced, checksummed, and hole-detected, every subscriber reimplements ByteNDB's gossip/back-link.
7. **Selective materialization.** Not every column, not every table. Heatmaps / cost models / Secondary Engine DDL (HeatWave, PolarDB `KEY COLUMN INDEX`, veDB). Full dual-format is how you lose on storage and write amp.
8. **Hybrid planner with a small search space.** Rule-based is what ships first (TiDB, HeatWave); cost/ML router is what veDB added when rules failed.
9. **Delete/update story in the column path.** Bitmaps (ByteHTAP), out-of-place + RID (PolarDB-IMCI), ReplacingMergeTree (ClickHouse CDC). In-place column updates do not exist at warehouse scale.

**Research-grade (not required to ship a layer):**

10. Per-level LSM layouts (LASER).
11. CSD-sparse hybrid pages (HaSiS) — only if the hardware exists in the target fleet.
12. PIM islands (Polynesia/PUSHtap).
13. Holistic scheduler that jointly moves freshness, threads, and placement (explicitly *open* in Zhang/Li 2024).
14. Distributed learned organization (Proteus-class) that is not an offline overnight job.

**Explicitly not required for excellence, and often harmful:**

- One query engine that is "good at both". ByteHTAP and veDB spent years *undoing* that idea.
- Zero-copy Kafka-on-Iceberg as a substitute for a row store (Vanlightly argues this is the wrong direction for Kafka; independently, Iceberg is not OLTP).
- Putting a SIMD runtime inside a local KV kernel.

---

## 10. Mapping onto PedraDB / Montanha

Doctrine already says the answer: **HTAP is a layer, not a kernel feature** ([doctrine](doctrine-primitives-and-api-layers.md), [RFC-0013](rfc/0013-montanhadb-product.md) lists "Full distributed SI/2PC / SQL HTAP" as out of scope / P2 strategy). [tidb-architecture.md](tidb-architecture.md) already maps TiFlash to "layers / out of core".

What this research changes is *which hooks the primitive should not paint itself into a corner on*.

### 10.1 Stay out of the kernel

| Do not put in `pedradb-core` | Why |
|------------------------------|-----|
| Columnar SIMD engine | Different physics; ClickHouse/DuckDB exist |
| Dual durable copy of every value | Write-amp tax with no product yet |
| SQL planner / hybrid router | veDB's lesson: this is a product, not a store |
| CSD-specific page format | Hardware-specific; HaSiS is a paper, not a fleet |

### 10.2 Hooks that make a later *excellent* layer cheap

These are option-preservation, same spirit as the sled-layer / object-storage notes.  
**Next useful RFC:** “WAL-as-product + LSN pin + learner apply” — **without** a columnar engine in core.

| Hook | Primitive it unlocks | Status today |
|------|----------------------|--------------|
| **WAL as a sequenced, checksummed, hole-detectable stream** | Raft learner, CDC, PolarDB-style redo subscriber, HTAS | WAL exists; treat export as a product surface (already a Must in [sql-lessons](sql-lessons-for-the-grail.md)); `ship_wal` / incremental backup exist |
| **Stable LSN / commit timestamp on every apply** | Freshness-by-catchup; ByteHTAP global LSN; `commit(olap=Applied)` | Internal seq exists; needs a *layer-visible* pin API |
| **SST that can grow a projection / CG without a second tree** | LASER-style per-level layout | SSTs are row KV; do not freeze "value is opaque blob" so hard that a CG SST is a fork; `value_ref`/vlog already separates large values |
| **Iterator that can merge by key across runs** | Merge-on-read of row delta + column base | Core already merges versions; keep the seam; planned: multi_get, scan project |
| **Delete/tombstone that a subscriber can turn into a bitmap** | ByteHTAP deletes | Tombstones exist |
| **Learner-shaped apply** (read-only apply of the same log into another store) | TiFlash / IMCI | Montanha Raft is the place, not PedraDB directory sharing (forbidden) |
| **Same-container quotas** | Isolation without second machine | Process-level: cache/CPU quotas OLTP vs OLAP apply/query |

### 10.3 Three honest product shapes on this stack

1. **Composed (ship-first, 2026 market).** PedraDB/Montanha as the OLTP primitive; WAL/CDC into ClickHouse / DuckDB / Parquet / Iceberg. This *is* HTAP from the user's point of view if freshness is specified. Matches ClickHouse/AWS/Snowflake's current shape. Lowest research risk. Aligns with “substitute CH role” via **path**, not reimplement CH.
2. **TiFlash-shaped layer.** Montanha-Store range Raft + a **learner** process that writes columnar files (same container or not). PedraDB stays row-LSM. Freshness-by-catchup at a read LSN; default product: **`Applied`** (visible), **`Durable`** dual-store opt-in. This is the NewSQL HTAP that RFC-0013 already sketched as later. Storage duplication of **projection** is expected and budgeted.
3. **LASER-in-kernel (only if a workload is proven).** Per-level CGs inside PedraDB ([§6.1](#61-real-time-lsm-trees--laser--the-pedradb-shaped-paper)). Attractive because we are already an LSM. Dangerous because it couples the kernel to a table schema (columns). Only justified if a layer *needs* in-process hybrid scans on the same files, and a second engine is measurably worse. Not a P0/P1.

(2) and (3) are not exclusive: LASER can be the *local* format of a learner.

### 10.3.1 Mapping the triangle to our product language

| Triangle vertex | Pedra / Montanha choice (default) |
|-----------------|-----------------------------------|
| Layout | OLTP = NSM/LSM (+ vlog); OLAP = column projection or external engine |
| Freshness | Named: lag p99 / `Applied` vs `None` vs `Durable` |
| Isolation | Prefer replica/learner or same-box **quotas**; not unbounded shared cache |

“Generic HTAP for all workloads” in the [platform doc](node-primitive-and-unified-platform.md) means: **all workloads get a correct path**, not one amp profile.

### 10.4 What we are *not* claiming

- That PedraDB will beat TiFlash or PolarDB-IMCI. Non-goal, same as RFC-0013.
- That HTAP is "dead". Composed architectures are winning *warehouse* share; in-process / cloud-native HTAP is still being published at VLDB/SIGMOD/FAST/ASPLOS in 2025.
- That object storage as primary OLTP is how you get HTAP. NVMe vs S3 latency (two to three orders of magnitude on 4 KB random) is why ClickHouse's managed Postgres stays on NVMe and why Databricks Lakebase needs a page-server cache. Our own [object-storage note](object-storage-as-substrate-possibility.md) already separates "cold substrate" from "commit path".

---

## 11. Bibliography (local copies)

Primary PDFs saved under `docs/references/` on 2026-08-12:

| File | Paper |
|------|-------|
| [`htap-databases-survey-arxiv2404.15670.pdf`](references/htap-databases-survey-arxiv2404.15670.pdf) | Zhang, Li, Zhang, Zhang, Feng. *HTAP Databases: A Survey*. arXiv:2404.15670, Apr 2024. Four architectures, five technique families, benchmarks, open problems. |
| [`realtime-lsm-htap-2022.pdf`](references/realtime-lsm-htap-2022.pdf) | Saxena, Golab, Idreos, Ilyas. *Real-Time LSM-Trees for HTAP Workloads*. 2022. LASER on RocksDB. |
| [`hasis-fast2025.pdf`](references/hasis-fast2025.pdf) | Huang et al. *HaSiS*. FAST 2025. CSD single-index. |
| [`tidb-raft-htap-vldb2020.pdf`](references/tidb-raft-htap-vldb2020.pdf) | Huang et al. *TiDB: A Raft-based HTAP Database*. PVLDB 13(12), 2020. |
| [`bytehtap-vldb2022.pdf`](references/bytehtap-vldb2022.pdf) | Chen et al. *ByteHTAP*. PVLDB 15(12), 2022. |
| [`vedb-htap-vldb2025.pdf`](references/vedb-htap-vldb2025.pdf) | Chen et al. *veDB-HTAP*. PVLDB 18, 2025. |
| [`polardb-imci-sigmod2023.pdf`](references/polardb-imci-sigmod2023.pdf) | Wang et al. *PolarDB-IMCI*. SIGMOD 2023 / arXiv:2305.08468. |
| [`polynesia-icde2022.pdf`](references/polynesia-icde2022.pdf) | Boroumand, Ghose, Oliveira, Mutlu. *Polynesia*. ICDE 2022. |
| [`pushtap-asplos2025.pdf`](references/pushtap-asplos2025.pdf) | Zhao et al. *PUSHtap*. ASPLOS 2025 / arXiv:2508.02309. |

Already in-tree and adjacent: [`wisckey-fast2016.pdf`](references/wisckey-fast2016.pdf), [`dostoevsky-sigmod2018.pdf`](references/dostoevsky-sigmod2018.pdf), [`monkey-sigmod2017.pdf`](references/monkey-sigmod2017.pdf).

### 11.1 Essential papers not yet copied locally

Fetch before relying on a secondary paraphrase:

- Özcan, Tian, Tözün. *Hybrid Transactional/Analytical Processing: A Survey*. SIGMOD 2017 tutorial. Historical baseline.
- Song, Zhou, Cui, Peng, Li. *A survey on hybrid transactional and analytical processing*. VLDB Journal 33:1485–1515, 2024. Independent survey (open access).
- Kemper, Neumann. *HyPer*. ICDE 2011.
- Färber et al. *The SAP HANA Database*. IEEE DEB 2012.
- Grund et al. *HYRISE*. PVLDB 2010.
- Arulraj, Pavlo, Menon. *Bridging the Archipelago*. SIGMOD 2016 (TILE).
- Alagiannis, Idreos, Ailamaki. *H2O*. SIGMOD 2014.
- Abebe, Lazu, Daudjee. *Proteus*. SIGMOD 2022; *Tiresias*. PVLDB 2022.
- Kim et al. *Diva: Making MVCC systems HTAP-friendly*. SIGMOD 2022.
- Li et al. *Mainlining Databases*. PVLDB 2020.
- Coelho et al. *HTAPBench*. ICPE 2017; Cole et al. *CH-benCHmark*; Kang et al. *OLxPBench* ICDE 2022; HATtrick; HyBench.
- Li, Pavlo et al. as cited above.

### 11.2 Strategy essays (not theorems)

- Jack Vanlightly. *Hybrid Transactional/Analytical Storage*. May 2024. [link](https://jack-vanlightly.com/blog/2024/5/2/hybrid-transactional-analytical-storage)
- Jack Vanlightly. *Can We Agree on a Storage/Workload Architecture Taxonomy?* June 2026. HTAP vs LTAP vs materializing vs shared tiering.
- Al Brown / ClickHouse. *Unifying OLTP and OLAP*. Mar 2026. Composed-architecture brief; useful numbers on NVMe vs S3; treat product claims as advocacy.
- Zhou Sun (Mooncake / Databricks). *HTAP is Dead* (May 2025). Position: composition not consolidation. Not a proof of impossibility.

---

## 12. Open problems (as of the sources above)

Stated as *open*, not *impossible*. What would close each one:

| Open problem | Source | What would close it |
|--------------|--------|---------------------|
| Distributed data organization (which columns, which node, which grain, which compression) without an overnight ML job | Zhang/Li §VI | An online, bounded-regret placer with a published cost model *and* a HyBench/HATtrick frontier |
| Holistic scheduler (workload + resources + freshness in one loop) | Zhang/Li §VI | A controller that moves queries *and* merge rate against a freshness SLO, measured on HATtrick |
| Hybrid plans that actually exchange intermediates across row and column engines | Zhang/Li §VI; veDB two-stage planner | A documented exchange format + TPC-H/HyBench win vs single-engine |
| Physical redo → foreign layout without a second log, *as a reusable primitive* | PolarDB-IMCI (solved in one product) | A paper/OSS that does it on a generic WAL, not PolarFS |
| Single-index on *commodity* SSD matching HaSiS freshness+amp | HaSiS (needs CSD) | Same design on vanilla NVMe without capacity blow-up — or a cheap CSD becoming default |
| All three triangle vertices on commodity hardware | Polynesia/PUSHtap (need PIM) | Independent reproduction on stock DRAM at ≥ warehouse scale |
| Whether HTAP eliminates ETL *in production apps* | Kishore 2025; ClickHouse 2026 | App-level study with named SLOs, not a microbench. Currently: sometimes freshness, rarely modeling (3NF vs star) |
| Cloud-native freshness when "log is the database" and compute is gone | Zhang/Li §VI; AlloyDB/SingleStore | Replay/catch-up SLO under compute scale-to-zero |
| Multi-model HTAP (graph, document) | Gart; Zhang/Li §VI | A pin + dual store that is not "ETL into Neo4j" |
| HSTAP (train ML on the same incremental stream) | Kang et al. | Feature-freshness metric + no OLTP collapse |

---

## 13. How to read this later

1. Triangle first (§3). If a design does not say which vertex it chose, it is incomplete.
2. LASER (§6.1) if the question is "what can an LSM kernel do".
3. PolarDB-IMCI + ByteHTAP (§6.3–6.4) if the question is "what a cloud/distributed layer should subscribe to".
4. HaSiS / Polynesia / PUSHtap (§6.2, §6.5) if the question is "what does the research ceiling look like".
5. §10 before writing any PedraDB code. HTAP does not start in `sst/table.rs`.
