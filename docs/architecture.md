# PedraDB architecture

A **transactional key-value storage engine** in Rust, designed as a foundation
for building databases.

## Mission

**Small surface. High speed. Absurd potential to build on top.**

The Rust embed space is crowded (fjall, SurrealKV, redb…). PedraDB does not win
by feature count. It wins on **power per unit of API**:

> Ordered key-value + multi-key ACID, **local library only** — so layers and
> future databases can treat it as bedrock.

Public mental model:

```text
open → begin → get/put/delete/range → commit
```

No server, no multi-node, no SQL, no indexes in core. Research LSM (WiscKey,
Monkey, Lazy Leveling) is **under the hood**, not a knob zoo.

Inspired by FoundationDB’s layer concept: **transactions enable abstraction**.
Unlike FDB, PedraDB stays **in-process** (no cluster tax on the local path).
Unlike RocksDB, multi-key ACID is **in the kernel**, not bolted on later.

Focus doctrine: [`positioning.md`](positioning.md).

## Architecture (layered)

**PedraDB is local only** — the library TiKV would embed instead of RocksDB
(or FDB instead of Redwood). **Multi-node is not PedraDB.** A future separate
DB product may embed PedraDB on every node.

```
  Future outer DB (NOT PedraDB)          PedraDB (this project)
  multi-Raft · PD · gRPC · 2PC           single process, one machine
       │                                      │
       │  each node links ──────────────────► │  library
       │                                      │  LSM + local ACID TX
```

Inside the PedraDB library (still one node):

```
┌──────────────────────────────────────────────────────────────┐
│  Local transactional API                                     │
│    get/put/delete/range · ACID · MVCC · OCC                  │
├──────────────────────────────────────────────────────────────┤
│  Local storage engine                                        │
│    WAL · MemTable · SST · value log · compaction             │
│    LSM + WiscKey + Monkey + Dostoevsky                       │
└──────────────────────────────────────────────────────────────┘
```

| Piece | Role | Analogy |
|-------|------|---------|
| **PedraDB** | Local engine (+ local TX) | **RocksDB** in TiKV / **Redwood** in FDB |
| **Outer multi-node DB** (later, other name) | Cluster product | **TiKV** / **FDB** |
| **Layers** | SQL, doc, graph… | TiDB / Record Layer |

**Public contract:** ordered KV + ACID transactions **on one machine** (embedded).
No Raft, no PD, no network service in this product.

Full clarification: [`architecture-refined.md`](architecture-refined.md).

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
- **No multi-node / no Raft / no cluster.** PedraDB is an **embedded local
  library** only — like RocksDB. Horizontal scale and cross-node TX belong to a
  **different product** that *embeds* PedraDB per node (like TiKV embeds
  RocksDB). Research on that outer shape lives in
  [`distribution-design.md`](distribution-design.md); it is **not** PedraDB
  scope.

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
│   ├── pedradb-core/      # L0 store + L1 db (modules split as they grow)
│   ├── pedradb-sim/       # deterministic simulation
│   ├── pedradb-oracle/    # RocksDB oracle for L0 tests only
│   └── pedradb-cli/       # CLI
├── docs/
│   ├── architecture.md
│   ├── architecture-refined.md   # L0/L1/L2 split (authoritative layering)
│   └── … (see docs/open-items.md index)
└── clippy.toml
```

`pedradb-core` is `#![forbid(unsafe_code)]`. WAL today is early **L0**.

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
