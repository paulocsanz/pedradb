# Databases and systems built on FoundationDB

> FoundationDB is intentionally **not** a SQL/document/graph product. It is an
> ordered transactional KV core. Everything richer is a **layer** (library or
> microserver) or an **internal system** that encodes its data model on top of
> FDB transactions.
>
> Primary catalog: [awesome-foundationdb](https://github.com/FoundationDB/awesome-foundationdb)
> (FoundationDB org). Cross-checked with Record Layer / Document Layer READMEs,
> FDB design docs, and public engineering posts (as of research 2026-08-11).

---

## How to read this list

Three different meanings of “built on FDB”:

| Kind | Meaning | Examples |
|------|---------|----------|
| **A. Public layer (OSS)** | Open-source data model / API on FDB | Record Layer, Document Layer |
| **B. Production product** | Real service/product whose storage/metadata is FDB | Snowflake metadata, CloudKit, Astra Serverless, Tigris |
| **C. Experimental / PoC** | Community demos, often unmaintained | etcd-on-FDB, Redis gateway, old SQL layer |

There is **no** large open-source “TiDB-on-FDB” style full SQL product that
became industry default. The successful pattern is: **company builds a private
layer for their product**, or uses Record Layer internally.

---

## A. Official / production-grade open-source layers

### 1. FoundationDB Record Layer

- **Repo:** [FoundationDB/fdb-record-layer](https://github.com/FoundationDB/fdb-record-layer)
- **What:** Record-oriented store + **relational / SQL interface** (JDBC) on FDB
- **Language:** Java library (not a separate storage process)
- **Features (from README):** nested types, arrays, vectors/ML embeddings,
  multi-tenant schema templates, planner with joins/agg, indexes maintained in
  transactions
- **Status:** Actively maintained, Maven Central — the main “real database
  layer” Apple/community ships

**Closest analogue:** what TiDB is to TiKV, but as a **library** on FDB, not a
MySQL wire-compatible mega-product.

### 2. FoundationDB Document Layer

- **Repo:** [FoundationDB/fdb-document-layer](https://github.com/FoundationDB/fdb-document-layer)
- **What:** Stateless microserver speaking **MongoDB wire protocol** (subset of
  MongoDB API ~3.0): CRUD, indexes, transactions
- **Storage:** All data in FDB KV
- **Guarantees:** Inherits FDB ACID; indexes always consistent with data
- **Status:** Official org project; subset compatibility (not full Mongo)

### 3. Design recipes (not full DBs)

Official docs ship “recipes” showing how to build:

- Tables, simple indexes, multimaps, queues, priority queues  
- Hierarchical documents, spatial indexes, vectors, blobs  
- Subspace indirection  

These are **patterns**, not packaged databases:
`https://apple.github.io/foundationdb/design-recipes.html`

### 4. Historical SQL layer (dead)

- Old **FoundationDB SQL layer** — archived / no longer developed  
  (listed under experimental in awesome-foundationdb: jaytaylor/sql-layer)  
- Pre-Apple commercial FDB had more productized multi-model packaging; after
  open-source, core + layers split more cleanly.

---

## B. Production systems known to run on FDB

These are not always “a database you install”; often FDB is the **substrate**
under a cloud product.

| System | Org | Role of FDB | Notes |
|--------|-----|-------------|-------|
| **CloudKit** | Apple | Structured storage for mobile apps | VLDB’18 paper *CloudKit: structured storage for mobile applications*; QuiCK queueing (SIGMOD’21). Record Layer lineage is deeply tied to Apple’s stack. |
| **Snowflake** | Snowflake | **Metadata** store (not the bulk warehouse data path) | Multiple eng talks/blogs: “How FoundationDB powers Snowflake metadata”; migration series on Medium; Markus Pilman talks on FDB at Snowflake |
| **Wavefront** | VMware | Metrics / monitoring backend | Listed in awesome-foundationdb production experience; wavefront-fdb-tailer tooling |
| **DataStax Astra DB Serverless** | DataStax / IBM | Serverless Cassandra-compatible cloud DB storage architecture | Blog: *How we built Astra DB Serverless on FoundationDB* |
| **Tigris** | Tigris Data | Global metadata / data layer for object storage | Blogs: building a database using FDB; data layer on FDB; meetup talks |
| **SkuVault** | SkuVault | Warehouse management | Public post on FDB layers (abdullin.com) |
| **Bigblue** | Bigblue | Multi-model DB on FDB | Engineering Medium post |
| **Adobe** | Adobe | Identity graph (and related FDB ops) | FDB Meetup talks 2024–2025 (identity graph, Spark connector, self-healing, RocksDB storage engine experiments) |
| **DeepSeek 3FS** | DeepSeek | Fire-Flyer File System | Listed as production layer in awesome-foundationdb |

### CouchDB note

Apache maintains **erlfdb** (Erlang bindings). There have been talks/experiments
about CouchDB + FDB; treat as **binding / exploration**, not “CouchDB runs on
FDB by default.”

---

## C. Community experimental layers (selected)

From awesome-foundationdb — **not** production guarantees:

| Layer | Model |
|-------|--------|
| OpenTick | Time-series |
| Nomure | Graph |
| JanusGraph adapter | Graph |
| fdb-etcd | etcd API |
| fdb-zk | ZooKeeper |
| fdb-gateway | Redis protocol |
| Lucene layer | Search |
| NBD / block device | Block storage |
| Hashicorp Vault PR | Secrets backend experiment |
| tsdb-layer (Artoul) | Time-series blog/demo |
| FQL | Query language experiment |
| Rhino | Low-latency KV in Rust (talk/repo) |

Useful as existence proofs that **many models map onto ordered TX KV** — same
thesis as PedraDB layers.

---

## What is *not* built on FDB

To avoid confusion with earlier PedraDB research:

| System | Built on |
|--------|----------|
| TiDB | TiKV + RocksDB + PD (not FDB) |
| CockroachDB | Pebble + multi-Raft (not FDB) |
| ScyllaDB | Custom LSM + Seastar (not FDB) |
| Cassandra | Own storage (not FDB) |
| MongoDB | WiredTiger (not FDB) |

People sometimes assume “transactional KV ⇒ FDB under everything.” False.
FDB is one substrate; multi-Raft+RocksDB (TiKV) and Pebble (CRDB) are others.

---

## Pattern that actually works in industry

```
                    ┌─────────────────────────────┐
                    │  Product-specific layer     │
                    │  (SQL, docs, metadata,      │
                    │   mobile sync, object index)│
                    └──────────────┬──────────────┘
                                   │ ACID multi-key TX
                    ┌──────────────▼──────────────┐
                    │  FoundationDB core          │
                    │  ordered KV only            │
                    └─────────────────────────────┘
```

**Successful users rarely publish a generic open-source DB.** They:

1. Keep a **private layer** (Snowflake metadata, CloudKit, Tigris metadata), or  
2. Use **Record Layer** as a shared library, or  
3. Ship a **wire-compatible microserver** (Document Layer ≈ Mongo subset).

That is exactly PedraDB’s intended ecosystem — with the difference that PedraDB
starts **embedded** (library in-process) before optional distribution.

---

## Implications for PedraDB

| Lesson | Takeaway |
|--------|----------|
| Layers work when core TX is rock-solid | FDB’s whole industry value is layers trusting ACID |
| Few open-source mega-layers become “the Postgres of FDB” | Record Layer is the strongest OSS attempt; Document Layer is subset |
| Big companies use FDB as **metadata / control-plane store** as often as as “the app DB” | Snowflake, Tigris — same role RocksDB often plays under Ceph metadata |
| Wire-compatible layers are a product strategy | Mongo protocol on FDB ≈ what Alternator is for Scylla / MySQL is for TiDB |
| PedraDB’s empty niche remains | **Embedded** TX pillar + later multi-Raft — FDB never offered a library-mode core |

---

## Sources

| Ref | Source |
|-----|--------|
| [Awesome] | github.com/FoundationDB/awesome-foundationdb |
| [Record] | github.com/FoundationDB/fdb-record-layer README |
| [DocLayer] | github.com/FoundationDB/fdb-document-layer README |
| [LayerConcept] | apple.github.io/foundationdb/layer-concept.html |
| [Recipes] | apple.github.io/foundationdb/design-recipes.html |
| [CloudKit] | VLDB’18 CloudKit paper (linked from awesome-fdb) |
| [Snowflake] | Public FDB Summit / Medium migration series (linked from awesome-fdb) |
| [Astra] | DataStax blog “How we built Astra DB Serverless on FoundationDB” |
| [Tigris] | blog.tigrisdata.com FDB posts (linked from awesome-fdb) |
