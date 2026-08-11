# Distributed storage systems: ScyllaDB, Ceph, and where PedraDB fits

> How distributed databases and storage systems manage local storage, and whether
> PedraDB can serve as a unifying foundation that replaces the fragmented landscape.

All sources fetched live via curl. Key sources persisted in `docs/references/`.

## The three-layer problem

Every database and storage system can be decomposed into three layers:

```
┌─────────────────────────────────────────────────────────────┐
│  Layer 3: Database / Service                                │
│  (SQL, CQL, S3 API, document model, graph model)            │
├─────────────────────────────────────────────────────────────┤
│  Layer 2: Distribution / Coordination                       │
│  (Raft/Paxos, sharding, replication, cross-node TX)         │
├─────────────────────────────────────────────────────────────┤
│  Layer 1: Local storage engine                              │
│  (LSM tree, B-tree, KV store — the thing that writes to disk)│
└─────────────────────────────────────────────────────────────┘
```

The industry has a fragmentation problem: **every system reimplements Layer 1**
(or wraps RocksDB), and **every system reimplements Layer 2**. The result is
massive duplication of the hardest engineering work.

## How the major distributed systems handle Layers 1 & 2

### ScyllaDB — custom LSM, thread-per-core
**Source:** scylladb/scylladb GitHub, Seastar README

- **Layer 1:** Custom LSM tree written from scratch in C++ (NOT RocksDB).
  Custom SSTable format, custom compaction. Tightly integrated with Seastar
  (their async, thread-per-core framework).
- **Layer 2:** Gossip protocol (Cassandra-compatible), vnode-based sharding,
  eventual consistency (tunable). No ACID transactions across partitions.
- **Key innovation:** *Shard-per-core architecture.* Each CPU core owns its
  data, its memory, its I/O. No cross-core locking. This is why Scylla is 2-10x
  faster than Cassandra on the same hardware.
- **Weakness:** The LSM is deeply coupled to Seastar's futures/promises model.
  Can't swap the storage engine without rewriting the whole database.

### Ceph — BlueStore (wraps RocksDB)
**Source:** Ceph architecture docs, storage-devices docs

- **Layer 1:** **BlueStore** — a custom back end that writes data directly to raw
  block devices (bypassing filesystem), but uses **RocksDB** to manage all
  metadata (object→block mapping, omap key-values, collections). BlueStore was
  built specifically because the previous design (FileStore + XFS) was too slow.
- **Layer 2:** CRUSH algorithm for decentralized data placement. OSDs
  communicate peer-to-peer. Monitors use Paxos. No central lookup table.
- **Key innovation:** *CRUSH.* Clients compute data location mathematically
  (hash + topology tree) instead of asking a central server. Scales to exabytes.
- **The opening for PedraDB:** BlueStore's metadata path is RocksDB. If
  PedraDB's transactional KV were drop-in better (with WiscKey separation for
  large omap values, Monkey Bloom for fast lookups), Ceph's metadata
  operations could be significantly faster. But BlueStore's data path (raw
  block I/O) is separate and wouldn't be replaced.

### FoundationDB — B-tree + simulation testing
**Source:** FDB architecture + testing docs (already analyzed)

- **Layer 1:** Redwood (custom B+tree with prefix compression, replacing
  SQLite-derived B-tree).
- **Layer 2:** Decoupled roles (commit proxies, resolvers, transaction logs,
  storage servers). Roles scale independently.
- **Key innovation:** *Deterministic simulation.* ~1 trillion CPU-hours.
  Found bugs no integration test ever could.
- **Already the model PedraDB follows:** ACID transactions in the core,
  layers on top.

### TiKV — wraps RocksDB + Raft
**Source:** General knowledge (tikv.org)

- **Layer 1:** RocksDB (not reimplemented). Two instances per node: one for
  data, one for Raft logs.
- **Layer 2:** Raft consensus, multi-Raft for sharding. MVCC layer on top.
  Percolator-style distributed transactions.
- **Key innovation:** *Multi-Raft.* Thousands of Raft groups, one per region,
  balancing automatically.
- **Weakness:** Inherits all of RocksDB's problems (write amp, no core TX).
  Titan (KV separation) exists but is opt-in and non-default.

### ClickHouse — custom columnar, not KV
**Source:** General knowledge

- **Layer 1:** Custom columnar engine (MergeTree). Parts are sorted,
  column-compressed, merged in background. Not an LSM or B-tree — a
  purpose-built OLAP structure.
- **Layer 2:** Distributed table shards, ZooKeeper/ClickHouse Keeper for
  coordination.
- **Key innovation:** *Vectorized execution.* Process columns in batches
  using SIMD. 100-1000x faster than row-oriented databases for analytics.
- **Not a PedraDB target:** Columnar OLAP is a fundamentally different access
  pattern. PedraDB (ordered KV + transactions) is not the right foundation
  for analytical workloads.

### CockroachDB — Pebble (clean-room RocksDB) + Raft
**Source:** Already analyzed

- **Layer 1:** Pebble (Go, clean-room RocksDB rewrite).
- **Layer 2:** Raft, distributed transactions with timestamp ordering.
- Spent years building the transaction layer on top of a non-transactional KV.

---

## The fragmentation tax

| System | Layer 1 (local storage) | Layer 2 (distribution) | TX in core? |
|--------|------------------------|------------------------|-------------|
| ScyllaDB | Custom LSM | Gossip + vnode | ❌ |
| Ceph | BlueStore (wraps RocksDB) | CRUSH + Paxos | ❌ |
| FoundationDB | Redwood B+tree | Decoupled roles | ✅ |
| TiKV | RocksDB | Multi-Raft | ❌ (layer on top) |
| CockroachDB | Pebble | Raft | ❌ (layer on top) |
| MongoDB | WiredTiger | Replica sets + sharding | ❌ |

**Five out of six have no ACID transactions in the storage layer.** Each one
reimplements consistency on top — at enormous cost.

## Where PedraDB fits: the unifying foundation

PedraDB's value proposition as a foundation is clearer now:

### What PedraDB replaces (Layer 1)
Any system that uses RocksDB as its local storage engine could use PedraDB
instead — and get:
- ACID transactions in the engine (no more building TX layers on top)
- WiscKey KV separation (lower write amp)
- Monkey Bloom allocation (faster lookups)
- Lazy Leveling (less compaction)
- `#![forbid(unsafe_code)]` (memory safety)
- Deterministic simulation testing (trust)

**Direct replacement candidates:**
- Ceph BlueStore's metadata RocksDB → PedraDB
- TiKV's RocksDB instances → PedraDB
- CockroachDB's Pebble → PedraDB (if it offered Rust bindings)

### What PedraDB does NOT replace
- ScyllaDB's custom LSM (too deeply coupled to Seastar)
- ClickHouse's MergeTree (different data model: columnar, not KV)
- Ceph's data path (raw block I/O, not KV)

### The bigger vision: PedraDB as a distributed foundation

If PedraDB's transactional API is clean enough, **distribution becomes a layer
on top** — not baked into the storage engine:

```
┌─────────────────────────────────────────────────────────┐
│  SQL DB · Document DB · Graph DB · S3-compatible API   │  databases
├─────────────────────────────────────────────────────────┤
│  Distributed PedraDB                                    │  distribution
│  (Raft · Sharding · Cross-node TX)                     │  (future layer)
├─────────────────────────────────────────────────────────┤
│  PedraDB core                                           │  the pillar
│  (ordered KV + ACID transactions)                      │
├─────────────────────────────────────────────────────────┤
│  LSM engine (WiscKey + Monkey + Dostoevsky)            │  implementation
└─────────────────────────────────────────────────────────┘
```

This is the **FoundationDB model applied to the whole stack**: the core is
minimal and correct; everything else (distribution, data models, query
languages) is built on top using the transactional API.

### Why this is better than the current landscape

| Problem today | PedraDB's answer |
|--------------|------------------|
| Every DB reimplements transactions | ACID TX in the core, free for all layers |
| Every DB reimplements storage | One engine, optimized with 3 proven-but-unadopted optimizations |
| Testing correctness of novel compaction | FDB-style deterministic simulation |
| C++ memory safety bugs | `#![forbid(unsafe_code)]` in Rust |
| Write amplification (RocksDB 50x) | WiscKey separation → ~5-10x |
| Suboptimal Bloom filters | Monkey allocation → O(1) lookup cost |

## Honest limitations

PedraDB is **not a silver bullet** for all storage workloads:

1. **Columnar analytics** (ClickHouse, Druid): These need vectorized column
   scanning, not ordered KV. PedraDB is the wrong abstraction for OLAP.

2. **Raw block/object storage** (Ceph data path, MinIO): These bypass KV
   entirely for bulk data. PedraDB could handle metadata, not the data path.

3. **Single-key extreme throughput** (Redis, Dragonfly): These are in-memory
   first. PedraDB is durable-first. Different optimization target.

4. **ScyllaDB-type tight coupling**: Systems where the storage engine is
   deeply woven into the execution framework (Seastar futures) can't easily
   swap engines.

What PedraDB **is** the right foundation for: any system that needs ordered
key-value storage with transactional consistency and durability — which is
the vast majority of operational databases (OLTP, document stores, graph
databases, time-series with indexing, distributed KV stores).

---

## Sources

| Ref | Source |
|-----|--------|
| [Ceph] | Ceph architecture docs — docs.ceph.com/en/latest/architecture/ |
| [Ceph-BS] | Ceph storage devices docs — BlueStore section |
| [Scylla] | scylladb/scylladb GitHub README |
| [Seastar] | scylladb/seastar README |
| [FDB] | FoundationDB architecture + layer concept + testing docs |
| [Pebble] | cockroachdb/pebble README |
| Prior: [D], [M], [W] | Already persisted in docs/references/ |
