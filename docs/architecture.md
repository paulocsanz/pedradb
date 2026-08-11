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

## Why PedraDB solves what FDB can't

FoundationDB has the right model (transactions in the core) but pays a
distributed-systems tax: 5-second transaction timeout, 10 MB transaction size
limit, 100 KB value limit, no long-running transactions. Every one of these
is a consequence of network round-trips, remote resolvers, and replication —
**not** a fundamental limit.

PedraDB is **embedded**: the transaction manager and storage engine share one
process. The commit path is a function call, not a network hop. This means:

| FDB limit | Why it exists | PedraDB |
|-----------|---------------|---------|
| 5 s timeout | Network latency across proxies/resolvers/storage | Local commit, no network |
| 10 MB tx size | Serialized over network to resolver | Direct MemTable write, no serialization |
| 100 KB value | Replicated 3× through proxies | WiscKey value log, written once |
| No long txns | Resolver memory + cluster-wide MVCC GC | Local MVCC, GC tied to local snapshots |

The **one** limit PedraDB shares with FDB is OCC conflict rate for long-running
concurrent transactions — a property of optimistic concurrency control itself,
not of any deployment topology.

Full analysis: [`docs/fdb-limitations-analysis.md`](fdb-limitations-analysis.md).

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
- **No built-in distribution.** PedraDB is an embedded engine. Distribution
  (Raft, sharding, cross-node transactions) is a separate concern — layers or
  wrappers add it when needed. The full distribution design is documented in
  [`distribution-design.md`](distribution-design.md).

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
│   ├── architecture.md                  # this file
│   ├── rocksdb-critiques-and-improvements.md
│   ├── engine-landscape-and-ideal-path.md
│   ├── distributed-systems-analysis.md
│   ├── distribution-design.md           # how embedded → distributed
│   ├── distribution-deep-research.md    # Percolator, Parallel Commits, PD, Raft
│   ├── scylladb-architecture.md         # Scylla: AP multi-master, Seastar, tunable CL
│   ├── tidb-architecture.md             # TiDB: SQL layer on TiKV+PD+TiFlash
│   ├── fdb-limitations-analysis.md
│   ├── open-items.md                    # living list of open items
│   └── references/                      # all primary sources
└── clippy.toml
```

`pedradb-core` is `#![forbid(unsafe_code)]`. The engine is pure Rust.

## Delivery roadmap

| Slice | Status | What it delivers |
|-------|--------|------------------|
| 0. WAL | ✅ done | Append-only crash-safe log (block format, masked CRC32C, recovery) |
| 1. InternalKey + MemTable | ⏳ next | Versioned keys as structs (`InternalKey`), sorted in-memory buffer (skip list or B-tree), fixed-size arena, sequence numbers, custom `Slice` type |
| 2. Transaction manager | 🔲 | Snapshot isolation via MVCC, optimistic concurrency control (OCC), conflict detection (interval tree of read/write ranges), atomic commit (WAL + memtable publish) |
| 3. Transactional API | 🔲 | Public `Transaction` API: `get`/`put`/`delete`/`range_read`, `begin`/`commit`/`abort`; conflict-range declarations |
| 4. SST format + flush | 🔲 | Block-based SST with Monkey Bloom allocation, WiscKey value log for large values, MemTable→SST flush, flushable batches for oversized transactions |
| 5. Get + range scan | 🔲 | Point lookup + merged iterator across MemTable ∪ SSTs, MVCC filtering by sequence number, range tombstone integration with block-skip |
| 6. Compaction | 🔲 | Lazy Leveling (Dostoevsky) as default, tiering in small levels + leveling in largest, invariant-based pacing (not static rate limit), analytical cost model for auto-tuning |
| 7. Version GC | 🔲 | Reclaim MVCC versions older than the oldest active snapshot; "snapshot too old" safety valve; WiscKey value-log garbage collection (scan + evict) |
| 8. Deterministic simulation | 🔲 | FDB-style simulation: disk/time/crash modeling, reproducible runs, randomized workloads + fault injection |
| 9. Cross-validation harness | 🔲 | Oracle diff: same workload vs RocksDB, snapshot comparison, format compatibility checks |

### Cross-cutting engineering items (span multiple slices)

These design decisions are resolved in principle but need concrete implementation
within their target slices:

| Item | Target slice | Status | Notes |
|------|-------------|--------|-------|
| `InternalKey` as struct (not encoded string) | 1 | designed | Pebble lesson [P2]; avoids alloc on every Seek |
| Fixed-size MemTable arena | 1 | designed | Pebble lesson [P2]; prevents OOM from large batches |
| Custom `Slice` type for values | 1 | designed | fjall lesson [F]; controls allocation strategy |
| Conflict detection (interval tree) | 2 | designed | Range-based, not per-key; reduces false aborts |
| Commit publish-queue (lock-free) | 2 | designed | Pebble lesson [P2]; no group-commit leader |
| Flushable batches for oversized txns | 4 | designed | Pebble lesson [P2]; batch becomes LSM level |
| Monkey Bloom allocation | 4 | designed | FPR ∝ run size, decreasing exponentially [M] |
| WiscKey value log | 4 | designed | Values written once, never compacted [W] |
| Range tombstones in merging iterator | 5 | designed | Pebble lesson [P2]; block-skip optimization |
| Static dispatch (enum+match) on hot path | 5 | designed | Pebble lesson [P2]; no trait objects in iterators |
| Lazy Leveling compaction strategy | 6 | designed | Dostoevsky [D]; needs simulation to validate |
| Version GC strategy | 7 | **open** | Stop-the-world vs incremental vs "snapshot too old" error |
| Value-log GC strategy | 7 | **open** | Online vs batch; interaction with compaction |
| Backpressure strategy | 7 | **open** | Explicit stall vs adaptive admission control |
