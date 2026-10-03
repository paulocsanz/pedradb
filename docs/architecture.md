# PedraDB architecture

A **transactional key-value storage engine** in Rust, designed as a foundation
for building databases. This is the single architecture document: what the
product is, what is deliberately outside it, and how the inside is layered.

## PedraDB is the local primitive (not the multi-node DB)

> **PedraDB does not ship multi-node.** It is the embedded local storage
> (and local TX) engine — the role **RocksDB plays for TiKV** and **Redwood
> plays for FoundationDB**.
>
> A **separate product** (a future FDB/TiKV-class database) will **link
> PedraDB as its per-node store**, the same way TiKV embeds RocksDB on every
> TiKV server. That outer DB owns Raft, PD, networking, cross-node TX.
> PedraDB owns the disk on one machine.

```text
  Future outer DB (NOT PedraDB)          PedraDB (this project)
  multi-Raft · PD · gRPC · 2PC           single process, one machine
       │                                      │
       │  each node links ──────────────────► │  library
       │                                      │  LSM + local ACID TX
```

| Product | Scope | Analogy |
|---------|--------|---------|
| **PedraDB** (this project) | **Local only** — library, one process, one disk | **RocksDB** inside TiKV; **Redwood** inside FDB |
| **Outer DB** (future, other name) | Multi-node TX KV (or SQL) | **TiKV** or **FDB** as a product |
| **Layers** on the outer DB | SQL, doc, graph… | TiDB on TiKV; Record Layer on FDB |

**PedraDB is never "the cluster."** The cluster **uses** PedraDB.

### Why this split

| If PedraDB tried to be TiKV/FDB | If PedraDB stays local (correct) |
|----------------------------------|----------------------------------|
| Mixes engine + consensus + product | Clear job: best local ordered KV (+ local TX) |
| Forces Raft/PD into the same roadmap as SST format | Outer DB can be designed later on a stable store |
| Competes with TiKV as a full distributed product | Competes with **RocksDB/Pebble** as the substrate |
| Harder to embed in random apps | Any app or any cluster can `use pedradb` |

TiKV's lesson: they **wrapped RocksDB** and then spent years on TX + multi-Raft
**outside** RocksDB. PedraDB's job is to be a **better thing to wrap** — so the
outer DB (when it exists) does not inherit RocksDB's write-amp, and gets
**local ACID** from the library instead of inventing everything on a mute
engine.

### What is inside PedraDB

Two **internal** layers, both **local**:

```text
┌─────────────────────────────────────────────────────────┐
│  PedraDB (single library — one node)                    │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Local transactional API                          │  │
│  │  begin/commit · MVCC · OCC · multi-key ACID       │  │
│  │  = so outer DB / apps don't reinvent local TX     │  │
│  ├───────────────────────────────────────────────────┤  │
│  │  Local storage engine                             │  │
│  │  WAL · MemTable · SST · value log · compaction    │  │
│  │  LSM + WiscKey + Monkey + Dostoevsky              │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

| Inside PedraDB | Outside PedraDB (other product / later) |
|----------------|----------------------------------------|
| WAL, SST, compaction | Raft / multi-Raft |
| Local get/put/scan | Network protocol, gRPC |
| Local multi-key TX (ACID on one machine) | Cross-node 2PC / Parallel Commits |
| Crash recovery on one disk | PD, TSO, rebalancing, membership |
| `#![forbid(unsafe_code)]` engine | Cluster operator, load balancer |

**Local TX ≠ multi-node.** ACID on one process is still "RocksDB-class product
surface with TX," not a cluster. The outer DB uses that when applying a Raft
log entry or serving a single-Region transaction entirely on one node.

### Mapping to the industry

| Role | In TiKV stack | In FDB stack | In our stack |
|------|---------------|--------------|--------------|
| Local engine | RocksDB | Redwood | **PedraDB** |
| Distributed KV product | TiKV | FoundationDB | **Future DB (name TBD)** |
| SQL / model layer | TiDB | Record Layer / apps | Layers on the future DB (or on PedraDB embedded) |

```text
TiKV node:     [ TiKV Raft + Percolator ] → [ RocksDB ]
FDB storage:   [ FDB roles / replication ] → [ Redwood ]
Our node:      [ Future DB Raft + TX  ] → [ PedraDB  ]
App embedded:  [ App code             ] → [ PedraDB  ]
```

### What this repo is not doing

- No multi-Raft / PD / cluster membership as PedraDB features.
- No "PedraDB L2 cluster" as part of the PedraDB product.
- No network pooler requirement, no Redwood port, no RocksDB wrap as the
  engine.
- Distribution research (kept in the development tree) informs what PedraDB
  must **support as a library** — apply batch, iterators, local TX, crash
  safety — not what PedraDB implements as a network service.

### What PedraDB must expose so a TiKV-like DB can use it

The outer DB needs roughly what TiKV needs from RocksDB:

| Capability | Why the outer DB needs it |
|------------|---------------------------|
| Durable write batch / atomic apply | Raft log apply → commit state machine |
| Ordered scan / snapshot read | Reads, compaction of logical state, TX |
| Local multi-key TX or atomic batch | Single-Region TX without distributed 2PC |
| Crash recovery | Node reboot |
| Controlled memory / flush | Backpressure, flow control |
| Stable disk format + versioning | Rolling upgrades of the outer DB |

Building **local TX into PedraDB** means the outer DB can treat "this Region's
leader commit" as a PedraDB transaction instead of hand-rolling MVCC on raw
SST put/get (the expensive TiKV path).

## Mission

**Justify use first.** Someone picks PedraDB because:

> Multi-key ACID + ordered KV, embedded, tiny API — correct updates without a
> cluster and without bolting TX onto RocksDB.

```text
open → begin → get/put/delete/range → commit
```

**Later** (trust and speed, not the pitch): deterministic simulation, research
LSM (WiscKey / Monkey / Lazy Leveling), optional outer multi-node product that
*embeds* PedraDB.

Local library only. No server, no multi-node in this product.

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
  RocksDB).

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

## Crate layout

```text
pedradb/
├── crates/
│   ├── pedradb-core/         # The engine: store + db + TX (#![forbid(unsafe_code)])
│   ├── pedradb-spec/         # Durability / atomicity predicates linked by rustc
│   ├── pedradb-posix/        # fdatasync · fallocate · fadvise (the only unsafe, with io-uring)
│   ├── pedradb-io-uring/     # Linux io_uring
│   ├── pedradb-sim/          # seeded I/O faults on the real recovery path
│   ├── pedradb-dst/          # deterministic simulation testing
│   ├── pedradb-oracle/       # RocksDB oracle for differential tests
│   ├── pedradb-ops/          # backup · WAL shipping · point-in-time restore
│   ├── rocksdb-compat/       # rust-rocksdb 0.22 API surface (migration path)
│   └── rocksdb-parity-bench/ # parity bench harness vs RocksDB
│
│   (distributed building blocks under development — store, raft, sql,
│   http, replication, and the name-shim experiments — live in the
│   development tree, not this mirror)
├── docs/                     # benchmarks · architecture · metrics · verification
└── formal/                   # Aeneas/Lean kernel sources
```

`pedradb-core` is `#![forbid(unsafe_code)]`. The only `unsafe` in the tree lives
in `pedradb-posix` and `pedradb-io-uring`.

## Delivery roadmap

| Slice | Status | What it delivers |
|-------|--------|------------------|
| 0. WAL | ✅ done | Append-only crash-safe log (block format, masked CRC32C, recovery) |
| 1. InternalKey + MemTable | ✅ done | Versioned keys as structs, sorted in-memory buffer, arena, sequence numbers |
| 2. Transaction manager | ✅ done | Snapshot isolation via MVCC, OCC, conflict detection, atomic commit |
| 3. Transactional API | ✅ done | Public `Transaction` API: get/put/delete/range, begin/commit/abort |
| 4. SST format + flush | ✅ done | Block-based SST, WiscKey value log, MemTable→SST flush |
| 5. Get + range scan | ✅ done | Point lookup + merged iterator, MVCC filtering, range tombstones |
| 6. Compaction | ✅ done | Lazy Leveling (Dostoevsky), invariant-based pacing |
| 7. Version GC | ✅ done | MVCC reclaim tied to oldest snapshot; "snapshot too old" valve |
| 8. Deterministic simulation | ✅ done | FDB-style sim: disk/time/crash modeling, reproducible runs |
| 9. Cross-validation harness | ✅ done | Oracle diff vs RocksDB |

**No slice is "add multi-node to PedraDB."** Trust hardening (formal kernels,
mutation campaigns, parity benches) continues across all slices.

### Cross-cutting engineering decisions

| Item | Status | Notes |
|------|--------|-------|
| `InternalKey` as struct (not encoded string) | done | Pebble lesson; avoids alloc on every Seek |
| Fixed-size MemTable arena | done | prevents OOM from large batches |
| Custom `Slice` type for values | done | controls allocation strategy |
| Conflict detection (interval tree) | done | range-based; reduces false aborts |
| Commit publish-queue (lock-free) | done | no group-commit leader |
| Flushable batches for oversized txns | done | batch becomes LSM level |
| Monkey Bloom allocation | done | FPR ∝ run size, decreasing exponentially |
| WiscKey value log | done | values written once, never compacted |
| Range tombstones in merging iterator | done | block-skip optimization |
| Static dispatch on hot path | done | no trait objects in iterators |
| Lazy Leveling compaction strategy | done | validated by simulation |
| Version GC strategy | done | snapshot-bounded reclaim |
| Value-log GC strategy | done | interaction with compaction settled |
| Backpressure strategy | open | explicit stall vs adaptive admission control |

## Decision log

| # | Decision | Status |
|---|----------|--------|
| 1 | PedraDB = **local library only** | **Accepted** |
| 2 | Multi-node = **separate product** that embeds PedraDB | **Accepted** |
| 3 | PedraDB role = RocksDB/Redwood, not TiKV/FDB product | **Accepted** |
| 4 | Local LSM (not Redwood port, not RocksDB wrap) | **Accepted** |
| 5 | Local TX in PedraDB (so outer DB isn't forced to bolt-on from zero) | **Accepted** |
| 6 | Distribution docs = research for outer DB, not PedraDB scope | **Accepted** |

## Related

- [`benchmarks.md`](benchmarks.md) — parity tables vs RocksDB default and fjall
- [`metrics.md`](metrics.md) — health model and observability
- [`verification.md`](verification.md) — the formal verification catalog
