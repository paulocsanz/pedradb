# etcd compared to every system in this research

> etcd is a **distributed, strongly consistent coordination / metadata KV** —
> historically “a distributed `/etc`.” It is **not** a general-purpose database
> and not an embedded storage engine. This doc places it against everything
> surveyed for PedraDB.
>
> Sources: etcd docs — [API guarantees](https://etcd.io/docs/v3.5/learning/api_guarantees/),
> [versus other stores](https://etcd.io/docs/v3.5/learning/why/) (fetched 2026-08-11).

---

## What etcd is (in one paragraph)

etcd is a **replicated ordered key-value store** built for **cluster
coordination**: configuration, service discovery, leader election, locks,
leases, Kubernetes control-plane state. Consistency is **strict
serializability** (linearizable KV by default). Durability is via Raft
consensus. Local storage is **bbolt** (B+tree, single-writer). The data model
is small keys/values, revisions, watches, and leases — **not** multi-GB
application datasets, SQL, or multi-master writes.

**Canonical production role:** Kubernetes API server → etcd.

---

## Architecture snapshot

```
Clients (gRPC)
      │
┌─────┴─────────────────────────────┐
│  etcd cluster (typically 3 or 5)  │
│  single Raft group for ALL data   │
│  one leader; followers replicate  │
└─────┬─────────────────────────────┘
      │
 local bbolt (B+tree) per member
```

| Property | etcd |
|----------|------|
| Language | Go |
| Consensus | **One Raft group** for the whole keyspace |
| Local engine | **bbolt** (B+tree), not LSM |
| Sharding | **No** (whole DB is one consensus domain) |
| Multi-write same key | **No** — only leader proposes |
| Multi-write different keys | Parallel only as concurrent Raft proposals on **same** leader |
| Consistency | **Strict serializable** / linearizable reads (default) |
| CAP | **CP** |
| Scale target | Metadata: GBs, not TBs of user data |
| Signature APIs | KV + **Watch** + **Lease** + multi-op Txn + concurrency (lock/election) |

Optional: **serializable** (stale) reads for performance — weaker than default.

---

## Master comparison matrix

### A. Core engines (embedded / storage libraries)

| | **etcd** | **RocksDB** | **Pebble** | **PedraDB (target)** | **Badger / fjall** | **LMDB** |
|--|----------|-------------|------------|----------------------|--------------------|----------|
| Role | Distributed coordination KV | Embedded LSM engine | Embedded LSM (CRDB) | Embedded **TX** KV pillar | Embedded LSM KV | Embedded B+tree |
| Process model | Server cluster | Library | Library | Library first | Library | Library |
| TX ACID multi-key | Yes (small txn) | No (or limited) | No | **Yes, core** | Partial (Badger SSI) | MVCC single-writer |
| Distribution | Built-in Raft | None | None | Future layer | None | None |
| Consistency | Strict serializable | N/A (local) | N/A | Serializable → strict when distributed | Local | Local |
| Use as app DB | ❌ wrong tool | ✅ under other DBs | ✅ under CRDB | ✅ foundation | ✅ | ✅ read-heavy |
| vs etcd | — | etcd uses bbolt, not RocksDB | — | PedraDB could *host* metadata like etcd *or* be general DB | — | etcd’s cousin at storage layer (B-tree) |

**Takeaway:** etcd is **not** in the RocksDB/PedraDB category. It is a
**finished distributed product** for a narrow job. PedraDB is an **engine**;
etcd is a **coordination service**.

### B. Distributed transactional / strong-consistency systems

| | **etcd** | **FoundationDB** | **TiKV** | **TiDB** | **CockroachDB** |
|--|----------|------------------|----------|----------|-----------------|
| Primary job | Metadata + coordination | General TX KV + layers | Distributed TX KV | MySQL SQL on TiKV | Distributed SQL |
| Data size sweet spot | Config / control plane (small) | App data + metadata | App data (huge) | App SQL data | App SQL data |
| Sharding | **None** (1 Raft) | Storage server ranges | Multi-Raft Regions | Via TiKV | Multi-Raft ranges |
| Write scale | **Leader bottleneck** | Global pipeline + scale roles | Horizontal via Regions | Horizontal | Horizontal |
| Multi-key TX | Yes (bounded) | Yes (with limits 5s/10MB) | Yes (Percolator) | Yes (SQL) | Yes (SQL) |
| Watches / leases | **First-class** | Not the product focus | Limited vs etcd | SQL triggers ≠ etcd watch | Changefeeds |
| SQL | No | Via layers (Record Layer) | No | **Yes MySQL** | **Yes Postgres** |
| Local engine | bbolt | Redwood B-tree | RocksDB | RocksDB (TiKV) | Pebble |
| Replace etcd? | — | Can store metadata (Snowflake does FDB; K8s uses etcd) | Can, but overkill / different API | No (wrong API) | No (wrong API) |
| Replace FDB/TiKV? | ❌ no sharding, no scale | — | — | — | — |

**Takeaway:** etcd shares **CP + strong consistency** with FDB/TiKV/CRDB/TiDB,
but **refuses horizontal data partitioning**. That is a feature (simpler
correctness for control plane), not a bug — until you put application data in
it.

### C. AP / multi-master / analytics / object storage

| | **etcd** | **ScyllaDB** | **Ceph** | **ClickHouse** |
|--|----------|--------------|----------|----------------|
| Consistency default | Strict serializable | Tunable / often eventual | Strong for object placement (CRUSH+Paxos monitors) | Eventual-ish analytics |
| Multi-master same key | No | **Yes (LWW)** | N/A (object replica sets) | N/A |
| Workload | Coordination | High-QPS wide-column | Object/block storage | OLAP scans |
| Overlap with etcd | None as app store | None | Monitors sometimes use etcd/other; data path unrelated | None |

**Takeaway:** Scylla and etcd are **opposites** on CAP and multi-write. Scylla
maximizes availability and throughput; etcd maximizes agreement.

### D. Products built *on* FDB (for context)

| Product | vs etcd |
|---------|---------|
| CloudKit / Record Layer | Full app data models on FDB — **much broader** than etcd |
| Snowflake metadata | Same *role class* as etcd (metadata), but FDB **scales** metadata far beyond single-Raft |
| Astra Serverless on FDB | Full DB product, not coordination |
| Tigris on FDB | Object metadata at scale |
| Document Layer | Mongo-like app DB |

**Takeaway:** companies that outgrow “one Raft group for all metadata”
often move **metadata to FDB** (or multi-Raft KV). etcd remains ideal when the
dataset stays small and the API (watch/lease/election) matches.

---

## Dimension-by-dimension

### 1. Purpose

| System | Why it exists |
|--------|----------------|
| **etcd** | Keep cluster configs and elections **correct** under partition |
| **PedraDB** | Be the **pillar** to build other databases (embedded TX KV) |
| **FDB** | Same pillar idea, always distributed |
| **TiKV** | Distributed TX KV for app data (and TiDB) |
| **TiDB / CRDB** | Distributed SQL for applications |
| **RocksDB / Pebble** | Fast local LSM under someone else’s system |
| **Scylla** | Extreme throughput NoSQL |
| **Ceph** | Distributed object/block/file |

### 2. Multi-write

| System | Multi-write? |
|--------|----------------|
| **etcd** | Single leader for **entire** keyspace |
| **FDB** | Global version order; not multi-master per key |
| **TiKV / CRDB / PedraDB dist** | Multi-leader **across shards**, single leader **per key** |
| **Scylla** | Multi-master **per key** (LWW) |

etcd is the **most centralized write path** of all systems listed: every write
goes through one Raft leader for the whole DB.

### 3. Consistency

| System | Model |
|--------|--------|
| **etcd** | Strict serializable (default linearizable) |
| **FDB, CRDB** | Strict / serializable |
| **TiKV/TiDB** | SI / RR strong |
| **PedraDB** | Serializable embedded → strict when distributed |
| **Scylla** | Tunable; often eventual |
| **RocksDB** | Local durability only |

etcd is in the **strongest consistency club** with FDB/CRDB — not with Scylla.

### 4. Scalability

| System | How it scales |
|--------|----------------|
| **etcd** | **Vertically** + few members (3/5). Official guidance: keep DB size modest (historically ~2–8 GB practical warnings; not a warehouse) |
| **TiKV / CRDB / PedraDB dist** | **Horizontally** via many Raft groups |
| **FDB** | Horizontally via storage servers + role scaling |
| **Scylla** | Horizontally via token ring + multi-master |
| **RocksDB** | Single node |

Putting Kubernetes-scale **application data** in etcd is an anti-pattern.
Putting **pod specs and leases** in etcd is the design point.

### 5. Features unique to etcd (among this list)

| Feature | etcd | Closest elsewhere |
|---------|------|-------------------|
| **Watch** (revision stream) | Native, core API | CRDB changefeeds; FDB layers DIY; not RocksDB |
| **Lease / TTL keys** | Native | Redis-ish; Scylla TTL; not FDB core |
| **Leader election helpers** | concurrency API | ZooKeeper; Raft DIY on others |
| **Revision history + compact** | Native MVCC window | FDB 5s window is different purpose |
| **gRPC control-plane API** | Yes | Consul/ZK category |

FDB/TiKV are **general stores**. etcd is a **coordination toolkit** with a KV
underneath.

### 6. Local storage engine choice

| System | Engine | Structure |
|--------|--------|-----------|
| etcd | bbolt | B+tree |
| RocksDB / Pebble / Badger / fjall / PedraDB | LSM | Write-optimized |
| FDB | Redwood | B+tree |
| LMDB | B+tree mmap | Read-optimized |
| Scylla | Custom LSM | Shard-per-core |

etcd picked B+tree because control-plane workloads are **read-heavy with
small values**, not LSM-style heavy sequential ingest.

---

## Where etcd sits in our four families

```
Family 1: Multi-Raft app data     TiKV, CRDB, PedraDB-distributed
Family 2: Decoupled roles         FoundationDB
Family 3: Embedded TX pillar      PedraDB-core, RocksDB (no TX), Pebble
Family 4: AP multi-master         Scylla

Family 0 (new label): Single-Raft coordination
          ★ etcd ★  (+ ZooKeeper, Consul in spirit)
```

etcd is **Family 0**: strong consistency, **no data sharding**, APIs for
**coordination**, tiny trusted dataset.

---

## Head-to-head narratives (the ones that matter)

### etcd vs FoundationDB

| | etcd | FDB |
|--|------|-----|
| Same | CP, strong consistency, ordered KV, multi-key TX | Same |
| Different | One Raft; tiny data; watch/lease product | Scales; layers for app models; no K8s-style API |
| Who wins for K8s control plane | **etcd** (API + ops ecosystem) | Overkill / wrong API |
| Who wins for multi-TB TX data | etcd loses | **FDB** |

Snowflake uses FDB for metadata at scale that would stress etcd’s single-Raft
design. Kubernetes uses etcd because the control plane **fits** etcd.

### etcd vs TiKV / TiDB / CockroachDB

| | etcd | TiKV/TiDB/CRDB |
|--|------|----------------|
| Same | Raft, CP, strong TX | Same family of guarantees |
| Different | No multi-Raft; no SQL product | Horizontal app data |
| Relationship | TiDB **used to** embed etcd for PD in older designs; modern PD has its own store path — coordination vs data remain separate concerns | App DB |

You would not store user tables in etcd; you would not use TiDB as the
Kubernetes lease store.

### etcd vs Scylla

Almost no overlap. etcd = never split-brain. Scylla = stay available, reconcile
later. Opposite poles of CAP.

### etcd vs RocksDB / PedraDB

| | etcd | RocksDB | PedraDB |
|--|------|---------|---------|
| etcd **contains** a local store | bbolt | — | — |
| Could PedraDB replace bbolt inside etcd? | Theoretically (research) | Sometimes people discuss | Same |
| Could PedraDB replace etcd the product? | Only with Raft + watch + lease + K8s API — a full product, not the core mission | No | No |

PedraDB’s mission is **pillar for databases**, not **Kubernetes coordination**.
A future layer could implement etcd-like semantics on PedraDB+Raft, similar to
community fdb-etcd experiments on FDB — that is a layer, not the core.

### etcd vs Ceph / ClickHouse

No meaningful competition. Different layers of the stack (coordination vs
object store vs OLAP).

---

## Decision guide: when to use which

| If you need… | Pick |
|--------------|------|
| K8s / service discovery / distributed locks / small config | **etcd** |
| Embedded ACID KV inside an app process | **PedraDB** (target) / Badger / LMDB |
| Fast local LSM under your own DB | **RocksDB / Pebble** |
| Distributed general TX KV, huge data | **FDB / TiKV** |
| Distributed SQL | **TiDB / CockroachDB** |
| Massive multi-master NoSQL QPS | **Scylla** |
| Object/block at exabyte scale | **Ceph** |
| Analytics scans | **ClickHouse** |

---

## One-sentence placements

| System | Sentence |
|--------|----------|
| **etcd** | Strongly consistent **coordination brain**, single Raft, small data. |
| **PedraDB** | Embedded **transactional pillar** for building DBs; distribution later. |
| **FDB** | Distributed transactional pillar + layers; scales past etcd for metadata. |
| **TiKV** | Distributed TX KV for **application** data (multi-Raft). |
| **TiDB** | MySQL SQL **layer** on TiKV. |
| **CockroachDB** | Postgres-compatible distributed SQL (Pebble + multi-Raft). |
| **RocksDB/Pebble** | Local LSM engines, no distribution, no core multi-key TX. |
| **Scylla** | AP multi-master high-throughput NoSQL. |
| **Ceph** | Distributed storage (objects/blocks), not a TX app KV. |
| **FDB products** (Snowflake meta, CloudKit, …) | Private layers on FDB; often same *job class* as etcd (metadata) at larger scale. |

---

## Sources

| Ref | Source |
|-----|--------|
| [etcd-guarantees] | etcd.io/docs/v3.5/learning/api_guarantees/ |
| [etcd-why] | etcd.io/docs/v3.5/learning/why/ (etcd versus other key-value stores) |
| Prior PedraDB docs | architecture, distribution-*, fdb-*, scylladb-*, tidb-*, foundationdb-layers-* |
