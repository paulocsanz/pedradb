# PedraDB architecture

A **transactional key-value storage engine** in Rust, designed as a foundation
for building databases.

## Mission

PedraDB is not "another KV store." It is a **pillar** upon which other databases
(SQL, document, graph, time-series) are constructed. The core exposes an ordered
key-value store with ACID transactions and nothing else — no data model, no
query language, no indexes. Database builders create layers on top, using
transactions to guarantee consistency between data and indexes.

This model is inspired by FoundationDB's layer concept, where the transaction
manifesto establishes that **transactions enable abstraction**: they make layers
composable, reliable, and efficient. Without ACID transactions at the core, every
database built on top must reinvent consistency — the hardest problem.

## Architecture (layered)

```
┌─────────────────────────────────────────────────────┐
│         SQL DB · Document DB · Graph DB · ...       │  user-built layers
├─────────────────────────────────────────────────────┤
│  PedraDB Transactional API                          │  the pillar
│    Transaction { get, put, delete, range_read }     │
│    snapshot isolation · ACID · conflict detection   │
├─────────────────────────────────────────────────────┤
│  MVCC + Sequence Numbers + Commit Pipeline          │  transaction layer
├─────────────────────────────────────────────────────┤
│  LSM Engine (implementation detail, swappable)      │
│    WiscKey KV separation · Monkey Bloom allocation  │
│    Dostoevsky Lazy Leveling                         │
├─────────────────────────────────────────────────────┤
│  WAL · MemTable · SST · Value Log · Compaction      │  storage
└─────────────────────────────────────────────────────┘
```

Everything below the transactional API is an **implementation detail**. The
public contract is: ordered KV + ACID transactions. The LSM engine underneath
incorporates three proven-but-unadopted academic optimizations (WiscKey, Monkey,
Dostoevsky) that no production engine has combined — because backward
compatibility, testing risk, and cold-start effort prevented it. PedraDB, as a
new project, faces none of those barriers.

## Why transactions at the core (not as a layer)

FoundationDB proved that ACID transactions are the **essential capability** that
allows developers to build data models reliably in a layer. The argument, from
FDB's Transaction Manifesto:

1. **Transactions enable abstraction.** A layer maintaining a secondary index
   can update both data and index in one transaction, guaranteeing consistency.
   Without transactions, you get eventual inconsistency between data and index.

2. **Transactions enable efficient representations.** Without multi-key
   transactions, developers denormalize ("embed") data into single documents/
   rows to achieve atomicity, resulting in larger, less efficient storage.

3. **Transactions enable flexibility.** When requirements change (read-only →
   read-write, single-key → multi-key), transactions make the difference between
   an easy change and a full re-architecture.

4. **Transactions are not expensive.** FDB's measured CPU overhead for
   transactional integrity is <10% of total system CPU. The cost is in engineering
   effort to build the system, not in runtime performance.

RocksDB's lack of core transactions forced CockroachDB and TiKV to build entire
distributed transaction layers on top — years of engineering each. PedraDB puts
transactions in the core so that database builders never have to solve
consistency themselves.

## Anti-features (deliberately NOT in the core)

Following the FoundationDB principle: the core is minimal so it can be as strong
as possible.

- **No data models.** No document model, no column families, no relational schema.
  The core is ordered key-value.
- **No query language.** No SQL, no JSON query, no scan filters. Layers provide these.
- **No indexes.** No secondary indexes, no bloom-filter-as-index. Layers maintain
  indexes using transactions (store index entries alongside data, update atomically).
- **No analytic frameworks.** No MapReduce, no streaming. Layers build these on top
  of range reads.
- **No built-in distribution.** PedraDB is an embedded engine. Distribution (Raft,
  sharding, cross-node transactions) is a separate concern — layers or wrappers
  add it when needed.

## Testing strategy: deterministic simulation

Borrowing from FoundationDB's greatest contribution: **deterministic simulation
testing**. An entire PedraDB instance runs inside a single-threaded simulation
that models disk, time, and crash semantics. Every run is perfectly reproducible.
Randomized workloads + fault injection find bugs that unit tests and even
integration tests cannot reach. FDB estimates they've run the equivalent of ~1
trillion CPU-hours of simulation.

This is especially critical because PedraDB adopts novel compaction strategies
(Lazy Leveling) that have no production precedent. You cannot trust a novel
compaction strategy without exhaustive simulation.

## LSM engine: the three optimizations nobody combined

| Optimization | Paper | Effect | Why nobody adopted it |
|-------------|-------|--------|-----------------------|
| KV separation | WiscKey (FAST'16) | Write amp O(T^L) → O(T·L) | Breaks existing SST format compat |
| Optimal Bloom allocation | Monkey (SIGMOD'17) | Lookup O(L·e^(-M/N)) → O(e^(-M/N)) | Changes filter format; needs analytical tuning |
| Lazy Leveling | Dostoevsky (SIGMOD'18) | Write amp ~10x less, same bounds | Fundamentally different compaction; needs sim testing |

Sources persisted in `docs/references/`.

## Crate layout

```
pedradb/
├── crates/
│   ├── pedradb-core/      # transactional KV engine (the pillar)
│   ├── pedradb-sim/       # deterministic simulation test framework
│   ├── pedradb-oracle/    # RocksDB bindings for cross-validation (dev only)
│   └── pedradb-cli/       # CLI
├── docs/
│   ├── architecture.md            # this file
│   ├── rocksdb-critiques-and-improvements.md
│   ├── engine-landscape-and-ideal-path.md
│   └── references/                # all primary sources
└── clippy.toml
```

`pedradb-core` is `#![forbid(unsafe_code)]`. The engine is pure Rust.

## Delivery roadmap

| Slice | Status | What it delivers |
|-------|--------|------------------|
| 0. WAL | ✅ done | Append-only crash-safe log (block format, masked CRC32C, recovery) |
| 1. InternalKey + MemTable | ⏳ next | Versioned keys as structs, sorted in-memory buffer, sequence numbers |
| 2. Transaction manager | 🔲 | Snapshot isolation, conflict detection, atomic commit (WAL + memtable) |
| 3. Transactional API | 🔲 | Public `Transaction` API: get/put/delete/range_read, begin/commit/abort |
| 4. SST format + flush | 🔲 | Block-based SST with Monkey Bloom, WiscKey value log, MemTable→SST |
| 5. Get + range scan | 🔲 | Point lookup + merged iterator across MemTable ∪ SSTs, MVCC filtering |
| 6. Compaction | 🔲 | Lazy Leveling (Dostoevsky), invariant-based pacing |
| 7. Deterministic simulation | 🔲 | FDB-style simulation: disk/time/crash modeling, reproducible runs |
| 8. Cross-validation harness | 🔲 | Oracle diff: same workload vs RocksDB, snapshot comparison |
