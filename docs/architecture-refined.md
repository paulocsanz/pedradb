# Refined architecture: local storage primitive → FDB/TiKV-class DB

> Clarifies the product layering after research. **FDB is comparable to TiKV**
> (distributed transactional KV), not to RocksDB (local engine). PedraDB
> therefore builds **bottom-up**: a local KV storage primitive first, then a
> transactional DB on top of it, then (optionally) a distributed FDB/TiKV-class
> system that **uses that primitive** the way TiKV uses RocksDB and FDB uses
> Redwood — but without FDB’s distributed taxes and without bolting TX onto a
> non-transactional engine after the fact.

---

## The insight (agreed)

```
RocksDB / Redwood / pedradb-store     =  local storage primitive
TiKV / FoundationDB / pedradb-db      =  distributed (or local) transactional KV product
TiDB / Record Layer / user layers     =  data models on top
```

| Role | FDB | TiKV | PedraDB |
|------|-----|------|---------|
| Local storage | **Redwood** (B+tree) | **RocksDB** (LSM) | **`pedradb-store`** (LSM + WiscKey/Monkey/Dostoevsky) |
| TX + product API | Whole cluster (roles) | Percolator on RocksDB | **`pedradb-txn` + API** (local first) |
| Distribution | Decoupled roles | Multi-Raft | **Multi-Raft later** (TiKV shape, not FDB roles) |
| Deploy today | Cluster only | Cluster only | **Library / embedded first** |

**PedraDB is not “pick TiKV or FDB.”** It is:

1. **Build a better local primitive** than RocksDB/Redwood (LSM research + Rust safety).
2. **Put TX on that primitive from day one** (avoid TiKV’s years bolting Percolator on RocksDB).
3. **Scale out like TiKV** (multi-Raft), with **FDB’s API philosophy** (minimal core, layers, strict consistency, simulation testing), **without FDB’s hard distributed limits** where embedded/local commit applies.

---

## Three layers (the architecture)

```
┌──────────────────────────────────────────────────────────────────┐
│  L3  User layers                                                 │
│      SQL · Document · Graph · etcd-like · object metadata · …    │
├──────────────────────────────────────────────────────────────────┤
│  L2  pedradb-cluster  (future)                                   │
│      multi-Raft · PD/TSO · Parallel Commits · shard-aware client │
│      “FDB/TiKV-class product, without FDB defects”               │
├──────────────────────────────────────────────────────────────────┤
│  L1  pedradb-db  (transactional KV)                              │
│      Transaction { get, put, delete, range }                     │
│      MVCC · OCC · commit pipeline · version GC                   │
│      usable embedded OR as the apply target under Raft           │
├──────────────────────────────────────────────────────────────────┤
│  L0  pedradb-store  (local KV storage primitive)                 │
│      WAL · MemTable · SST · value log · flush · compaction       │
│      crash recovery · iterators · no multi-key TX required here  │
│      “what RocksDB/Redwood are to TiKV/FDB”                      │
└──────────────────────────────────────────────────────────────────┘
```

### L0 — `pedradb-store` (local primitive)

**Job:** durable, ordered, single-node key-value **storage** — the thing that
owns the disk format and recovery.

| Is | Is not |
|----|--------|
| WAL, MemTable, SST, compaction | Network, Raft, SQL |
| Point get + range iterate | Multi-key ACID (that’s L1) |
| Crash-safe local durability | Cluster membership |
| Swappable in principle | User-facing product alone |

**Design choices (locked):**

- **LSM**, not B-tree (write-heavy OLTP; WiscKey/Monkey/Dostoevsky apply to LSM).
- Clean-room Rust, `#![forbid(unsafe_code)]`.
- RocksDB as **oracle only** (format/behavior tests), not linked into the engine.
- Not a Redwood port — different structure on purpose.

**Analogy:** RocksDB to TiKV, Redwood to FDB, bbolt to etcd.

**Why build our own instead of wrapping RocksDB?**

| Wrap RocksDB | Own `pedradb-store` |
|--------------|---------------------|
| Instant engine | Control of format + compaction |
| Inherit write-amp, uniform Bloom, C++/FFI | WiscKey + Monkey + Lazy Leveling from day 1 |
| TX still bolted on later (TiKV path) | L1 designed **with** L0 (seqnum, InternalKey, batch-as-level) |
| PedraDB becomes “another TiKV” | PedraDB can be embedded pillar **and** cluster substrate |

### L1 — `pedradb-db` (transactional KV)

**Job:** ACID ordered KV API on top of L0 — the **pillar** for layers.

| Is | Is not |
|----|--------|
| `begin/commit/abort`, snapshot, OCC | SQL, indexes, documents |
| MVCC filtering over L0 versions | Distribution (optional L2) |
| Embedded library for apps | Bound to multi-node deploy |

**Critical design rule:** L1 must be usable in **two** modes without forking:

1. **Embedded:** app links L1; commit = local WAL + memtable publish.  
2. **Replica apply path:** Raft (L2) proposes a batch → L1/L0 apply is the
   state machine. Local TX semantics stay consistent.

This is how we avoid TiKV’s historical pain: they had L0 (RocksDB) without L1,
so TX became a huge distributed layer. We have L1 **before** L2.

### L2 — `pedradb-cluster` (FDB/TiKV-class, future)

**Job:** horizontal scale + HA for the same L1 API.

| Choice | Decision |
|--------|----------|
| Shape | **Multi-Raft** (TiKV/CRDB), not FDB decoupled roles |
| Cross-shard TX | Parallel Commits (not classic 2PC latency) |
| Consistency | Strict serializable / CP |
| Clock | TSO or HLC (open) |
| Connection model | Shard-aware gRPC client; **not** PgBouncer-on-primary |
| FDB defects | No 5s/10MB/100KB as **hard** embedded limits; soft network bounds only when distributed |

**Analogy:** TiKV’s *distribution*, FDB’s *product contract*, our L0+L1 as substrate.

### L3 — user layers

Unchanged philosophy: SQL, document, graph, “etcd-like coordination,” etc.
built with multi-key TX. See TiDB-on-TiKV and Record Layer-on-FDB as existence
proofs.

---

## What we are building *now* vs later

| Phase | Delivers | Maps to |
|-------|----------|---------|
| **Now** | WAL → MemTable → SST → get/scan → compaction | **L0 store** (+ enough versioning for L1) |
| **Next** | TX manager + public Transaction API | **L1 db** |
| **Then** | Simulation + oracle | Trust L0+L1 |
| **Later** | multi-Raft + PD + distributed TX | **L2 cluster** |
| **Optional** | SQL / other | **L3** |

Current code (`pedradb-core` WAL) is the start of **L0**. Going forward, the
crate split should make L0 vs L1 explicit (even if monorepo keeps them as
modules first):

```
crates/
  pedradb-store/     # L0 local primitive (or module store::)
  pedradb-db/        # L1 transactional API (or module db:: / tx::)
  pedradb-cluster/   # L2 future
  pedradb-sim/
  pedradb-oracle/    # RocksDB oracle for L0 behavior
  pedradb-cli/
```

Until a clean split is painful, `pedradb-core` may host both `store` and `tx`
modules — but **architecturally** they stay separate boundaries.

---

## Interface sketch (boundaries)

### L0 store (primitive)

```text
Store::open(path, opts) -> Store
Store::put(key, value) / delete(key)     # single-key or batch, no multi-key ACID
Store::get(key) -> Option<value>
Store::scan(start, end) -> Iterator
Store::write_batch(batch) -> durable after sync policy
Store::flush() / compact() / sequence_number()
```

Versioned internal keys (`user_key + seq + kind`) live here or at L0/L1
boundary — same as RocksDB InternalKey. L1 assigns seqnums on commit.

### L1 db (transactions)

```text
Db::open(...) -> Db          # owns a Store
Db::begin() -> Transaction
Transaction::get/put/delete/range
Transaction::commit() / abort()
# commit: conflict check → assign seq → WAL+memtable (or Raft propose in L2)
```

### L2 cluster (later)

```text
Cluster::open(...) 
# same Transaction API; commit may 2PC across Regions
# each Region’s apply path calls into L1/L0 on that node
```

---

## Explicit non-goals at each layer

| Layer | Will not do |
|-------|-------------|
| L0 | Multi-key TX, SQL, network, multi-tenancy product features |
| L1 | SQL, secondary indexes, multi-Raft (may expose hooks for apply) |
| L2 | Replace L0 with RocksDB “for speed” as default path; FDB role zoo |
| L3 | Living inside L0/L1 crates |

---

## How this fixes the FDB / TiKV / RocksDB confusion

| Question | Answer |
|----------|--------|
| Is PedraDB like RocksDB? | **L0 yes** (local engine role). |
| Is PedraDB like FDB? | **L1+L2 product contract yes** (TX KV + layers). |
| Is PedraDB like TiKV? | **L2 shape yes** (multi-Raft); **L0+L1 better integrated** than RocksDB+bolt-on TX. |
| Do we reimplement FDB in Rust? | **Reimplement the *class of system*** (distributed TX KV without FDB defects), not a line-by-line FDB clone (no Flow, no role zoo required). |
| Do we need Redwood *and* RocksDB? | **Neither as dependency.** One primitive: our LSM store. Redwood/RocksDB are *reference roles*, not crates we must ship. |
| PgBouncer? | Not for L0/L1/L2 KV. Only maybe a future SQL layer frontend. |

---

## Roadmap alignment

Existing slices 0–7 map cleanly:

| Slices | Layer |
|--------|-------|
| 0–1, 4–6 (WAL, MemTable, SST, get/scan, compaction) | **L0 store** |
| 2–3, 7 (TX manager, API, version GC) | **L1 db** |
| 8–9 sim + oracle | Trust L0+L1 |
| Future | **L2 cluster** |

**Next engineering step remains Slice 1** (InternalKey + MemTable) — solid L0
foundation. L1 TX sits on L0 once versions and batches exist.

---

## Decision log (this refinement)

| # | Decision | Status |
|---|----------|--------|
| R1 | Explicit L0 store / L1 db / L2 cluster split | **Accepted** |
| R2 | L0 = LSM (not Redwood B-tree port) | **Accepted** |
| R3 | L2 = multi-Raft (not FDB roles) | **Accepted** (prior) |
| R4 | L1 before L2 (TX before distribution) | **Accepted** |
| R5 | Do not wrap RocksDB as L0 default | **Accepted** |
| R6 | Oracle RocksDB for L0 tests only | **Accepted** |
| R7 | Crate split store/db when modules stabilize | **Deferred** (logical boundary now) |

---

## Related docs

- [`architecture.md`](architecture.md) — mission, anti-features, roadmap  
- [`distribution-design.md`](distribution-design.md) — L2 multi-Raft  
- [`fdb-limitations-analysis.md`](fdb-limitations-analysis.md) — defects L2 must not reintroduce as hard limits  
- [`engine-landscape-and-ideal-path.md`](engine-landscape-and-ideal-path.md) — why LSM + three papers  
- [`tidb-architecture.md`](tidb-architecture.md) — proof of L3 on a TX KV  
