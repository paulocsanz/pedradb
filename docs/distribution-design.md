# PedraDB distribution: how an embedded engine becomes a distributed database

> The central question: PedraDB is embedded (library, single-process). How do
> we distribute it across multiple nodes? Is it multi-write? Eventually
> consistent? This document maps the entire design space, documents every nuance,
> and identifies the path PedraDB should take.

All claims sourced from primary documentation (FDB, TiKV/PingCAP, CockroachDB)
fetched live. Key sources persisted in `docs/references/`.

---

## The fundamental insight

**Distribution is not a storage-engine concern — it is a layer on top.**

PedraDB's core contract is: *ordered KV + ACID transactions*. This contract is
the same whether the engine runs embedded in one process or as part of a
distributed cluster. The distributed layer wraps the embedded engine, adding:

1. **Replication** (data exists on multiple nodes)
2. **Sharding** (data is split across nodes)
3. **Consensus** (nodes agree on the order of writes)
4. **Distributed transactions** (transactions spanning multiple shards)

Every system that distributes an embedded engine — TiKV over RocksDB,
CockroachDB over Pebble, etcd over BoltDB — does it the same way: **wrap the
embedded engine with a consensus protocol (Raft) and a transaction layer**.

---

## The two proven distribution architectures

### Architecture A: Shared-Nothing Multi-Raft (TiKV / CockroachDB model)

```
┌──────────────────────────────────────────────────────────────┐
│  Client                                                      │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  Transaction Coordinator (Percolator / Timestamp Oracle)│  │
│  └──────────────────────┬─────────────────────────────────┘  │
└─────────────────────────┼────────────────────────────────────┘
                          │ network (gRPC)
          ┌───────────────┼───────────────┐
          │               │               │
    ┌─────┴─────┐   ┌─────┴─────┐   ┌─────┴─────┐
    │  Node A   │   │  Node B   │   │  Node C   │
    │           │   │           │   │           │
    │ Raft Group│   │ Raft Group│   │ Raft Group│
    │  Region 1 │   │  Region 3 │   │  Region 5 │
    │  (leader) │   │  (leader) │   │  (leader) │
    │           │   │           │   │           │
    │ Raft Group│   │ Raft Group│   │ Raft Group│
    │  Region 2 │   │  Region 4 │   │  Region 6 │
    │ (follower)│   │ (follower)│   │ (follower)│
    │           │   │           │   │           │
    │ PedraDB   │   │ PedraDB   │   │ PedraDB   │
    │ (embedded)│   │ (embedded)│   │ (embedded)│
    └───────────┘   └───────────┘   └───────────┘
```

**How it works:**

1. **Data is sharded into Regions** (TiKV) or Ranges (CockroachDB) — contiguous
   key ranges, typically 64–256 MB each.

2. **Each Region is an independent Raft group** with 3 replicas (default).
   One replica is the **leader** and serves reads + writes for that Region.
   This is **multi-Raft**: thousands of independent Raft groups, one per Region.

3. **The embedded engine (PedraDB/RocksDB/Pebble) stores data locally.** Each
   node runs one PedraDB instance that holds many Regions. The Raft log is
   separate from the data log (TiKV uses a separate RocksDB instance for Raft
   logs; PedraDB would use a separate WAL).

4. **Writes go through Raft consensus:** client writes to the Region leader →
   leader appends to Raft log → replicates to followers → once majority
   acknowledges, the write commits → leader applies it to the local PedraDB.

5. **Distributed transactions use a coordinator** (2PC / Percolator):
   - A transaction touching multiple Regions picks one key as the **primary
     lock**.
   - Phase 1 (PREWRITE): lock all keys, write new values tentatively.
   - Phase 2 (COMMIT): write the primary lock's commit record, then
     asynchronously commit all secondary keys.
   - If the coordinator dies mid-transaction, other nodes can resolve the
     lock by checking the primary's status.

6. **Placement Driver (PD)** / CockroachDB's range allocator: a separate
   component that tracks which node owns which Region, splits/merges Regions
   as they grow/shrink, and rebalances for load.

**Consistency model:** **Strict serializable** (linearizable + serializable).
Every read sees the latest committed write, and the transaction history is
equivalent to a serial execution. This is the strongest possible model.

**Multi-write?** **Yes and no:**
- Within a single Region: **single-leader**. Only the Raft leader accepts
  writes. Followers are read-only (or serve stale reads if configured).
- Across Regions: **multi-leader**. Different Regions have different leaders,
  potentially on different nodes. So multiple nodes accept writes
  simultaneously — but each write goes to exactly one leader for that key
  range.
- **Not multi-master / not multi-writer per key.** Two nodes cannot both write
  the same key independently. There is always exactly one leader per Region.

**Failure handling:** If the leader of a Region dies, Raft elects a new leader
among the followers. Unavailable for a brief window (election, typically
< 1 second). If a minority of nodes partition, the majority side continues.
If the majority is unreachable, the cluster stops accepting writes (CP
behavior).

### Architecture B: Decoupled Roles (FoundationDB model)

```
┌──────────────────────────────────────────────────────────────┐
│  Client                                                      │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  Client library (reads go directly to storage servers)  │  │
│  └──────────────────────┬─────────────────────────────────┘  │
└─────────────────────────┼────────────────────────────────────┘
                          │
         ┌────────────────┼────────────────┐
         │                │                │
   ┌─────┴──────┐  ┌─────┴──────┐  ┌──────┴──────┐
   │ GRV Proxy  │  │Commit Proxy│  │  Resolver   │
   │ (read      │  │ (commit    │  │ (conflict   │
   │  versions) │  │  versions) │  │  detection) │
   └────────────┘  └─────┬──────┘  └─────────────┘
                          │
                   ┌──────┴──────┐
                   │ Tx Log       │
                   │ (durability) │
                   └──────┬──────┘
                          │
          ┌───────────────┼───────────────┐
          │               │               │
    ┌─────┴─────┐   ┌─────┴─────┐   ┌─────┴─────┐
    │ Storage   │   │ Storage   │   │ Storage   │
    │ Server A  │   │ Server B  │   │ Server C  │
    │ (shard 1) │   │ (shard 2) │   │ (shard 3) │
    │ PedraDB   │   │ PedraDB   │   │ PedraDB   │
    └───────────┘   └───────────┘   └───────────┘
```

**How it works:**

1. **Roles are decoupled, not co-located.** Unlike multi-Raft where each node
   runs everything, FDB separates: commit proxies (assign commit versions),
   resolvers (conflict detection), transaction logs (durability), storage
   servers (hold data). Each role scales independently.

2. **Reads bypass the transaction system.** A client gets a read version from a
   GRV proxy, then reads **directly** from the storage server that owns the key.
   Reads scale linearly with the number of storage servers.

3. **Writes go through the full pipeline:**
   - Client sends transaction (reads + writes) to a commit proxy.
   - Commit proxy asks the master for a commit version.
   - Resolver checks for conflicts (holds last 5s of writes in memory).
   - Transaction log makes it durable (fsync'd to disk).
   - Transaction log forwards mutations to storage servers.

4. **Resolvers hold only 5 seconds of write history.** This is the root cause
   of FDB's 5-second timeout (see `fdb-limitations-analysis.md`).

5. **Coordinators** (small set of Paxos-based servers) manage cluster
   membership and leader election. Not involved in transaction processing.

**Consistency model:** **Strict serializable.** Same guarantee as Architecture A.

**Multi-write?** **No.** There is a single master that assigns commit versions.
Writes are serialized through commit proxies. Multiple commit proxies exist for
throughput, but they all get versions from the same master — so writes are
globally ordered.

**Failure handling:** If the master fails, coordinators trigger a recovery
(generational change). The cluster is briefly unavailable for writes (typically
< 1 second). Data remains available for reads.

---

## Architecture comparison: which path for PedraDB?

| Dimension | A: Multi-Raft (TiKV/CRDB) | B: Decoupled (FDB) |
|-----------|---------------------------|---------------------|
| **Complexity** | High (Raft per Region + 2PC for cross-Region TX) | Very high (separate roles, recovery protocol) |
| **Read scaling** | Good (leader serves reads; followers optional stale reads) | Excellent (reads bypass TX system, go direct to storage) |
| **Write scaling** | Good (more Regions → more leaders → more write parallelism) | Good (more commit proxies/resolvers/tx logs) |
| **Latency** | 1 Raft round-trip per write (~1-5ms LAN) | 3+ network hops per write (proxy→resolver→txlog→storage) |
| **Failure recovery** | Per-Region leader election (~100ms-1s per affected Region) | Global recovery (master regeneration, ~1-5s) |
| **Embedded engine role** | Stores Region data locally | Stores shard data locally |
| **Transactional KV in core?** | Not needed (TX layer bolted on) | **Already there** — PedraDB provides it |
| **Testability** | Hard (distributed consensus + 2PC + failure modes) | FDB's simulation testing is the gold standard |

### The key advantage PedraDB has in the distributed setting

Both TiKV and CockroachDB bolt transaction layers on top of non-transactional
embedded engines. This is enormously complex — CockroachDB spent years building
their transaction layer.

**PedraDB already has ACID transactions in the embedded core.** When
distributed, the question is not "how do we build distributed transactions?"
but "how do we **extend** existing local transactions across nodes?"

This is a fundamentally simpler problem:

- **Within a Region (single node):** PedraDB's local transactions handle
  everything. No distributed coordination needed.
- **Across Regions:** The distributed layer needs to coordinate transaction
  commits across multiple PedraDB instances. But each instance already provides
  local ACID — the distributed layer only adds the 2PC coordination on top.

### Recommended path: Multi-Raft (Architecture A), for three reasons

1. **Simpler to build.** Raft is well-understood, well-specified, and has
   multiple production-quality Rust implementations (`openraft`, `raft-rs`).
   FDB's decoupled architecture is bespoke and extremely complex to reproduce.

2. **Better latency.** One Raft round-trip per write (1-5ms) vs. FDB's 3+
   network hops. For an embedded engine that prizes local performance, this
   matters.

3. **Each Region is already a mini-database.** PedraDB's transactional API maps
   perfectly to a Region: a Region is a contiguous key range, and PedraDB can
   serve transactions within that range locally, with zero distributed
   coordination.

---

## Nuance 1: Consistency models — what guarantees does PedraDB offer?

### When embedded (single node)

| Property | Guarantee |
|----------|-----------|
| Isolation | Serializable (via OCC + MVCC) |
| Durability | Writes survive process crash (WAL + fsync) |
| Linearizability | N/A (single node — all operations are inherently linearized) |

### When distributed (multi-Raft)

| Property | Guarantee | How |
|----------|-----------|-----|
| Isolation | **Strict serializable** | Each transaction gets a globally ordered timestamp (timestamp oracle / HLC); reads see the latest committed version at that timestamp |
| Durability | Writes survive node failure | Raft majority replication before commit |
| Linearizability | **Yes** | Raft leader serves reads; linearizable reads use ReadIndex or lease-based reads |
| Consistency under partition | **CP** (consistent + partition-tolerant) | Raft majority must be available; minority partition rejects writes |

### PedraDB is NOT eventually consistent

PedraDB (embedded or distributed) provides **strict serializability** — the
strongest consistency model. This is the same guarantee as FoundationDB and
CockroachDB.

Eventual consistency (like Cassandra, ScyllaDB in default mode, DynamoDB)
trades correctness for availability during partitions. PedraDB does not make
this trade: it chooses consistency over availability (CP, in CAP terms).

As FDB's CAP analysis notes: *"A database can provide strong consistency and
system availability during network partitions. The common belief that this
combination is impossible is based on a misunderstanding of the CAP theorem."*
The "availability" PedraDB gives up is CAP-availability (every node can serve
every request), not practical availability (the system stays up for clients
that can reach a majority).

---

## Nuance 2: Multi-write — who accepts writes?

This is the most nuanced question. There are several meanings of "multi-write":

### 2a. Multi-node write acceptance (different keys on different nodes)

**Yes.** In multi-Raft, each Region has its own leader. Different Regions can
have leaders on different nodes. So:

- Node A is leader for Region 1 (keys `aaa`–`fff`)
- Node B is leader for Region 2 (keys `ggg`–`mmm`)

A client can write to `aaa` (goes to Node A) and `ggg` (goes to Node B)
**simultaneously**. Both writes proceed in parallel. This is genuine multi-node
write parallelism.

### 2b. Concurrent writes to the same key from different nodes

**No.** Only the Region leader accepts writes for a given key. If a client
connects to a follower and tries to write, the follower redirects to the leader.
This is fundamental to Raft — a single leader per key range ensures total order.

### 2c. Multi-datacenter writes (active-active)

**Configurable, but not default.** Three options:

| Mode | How it works | Trade-off |
|------|-------------|-----------|
| **Single-leader per Region** (default) | Each Region's Raft leader is in one datacenter. Cross-DC writes incur WAN latency. | Strong consistency, high write latency for cross-DC |
| **Geo-distributed Raft** | Raft replicas span datacenters. Leader is in DC1; writes replicate to DC2/DC3. | Strong consistency, write latency = slowest DC in majority |
| **Multi-region active-active** (Spanner/CockroachDB model) | Different key ranges have leaders in different regions. Write to a key goes to that key's leader region. | Strong consistency, low latency for region-local writes, cross-region TX is slower |

PedraDB would support mode 1 and 2 naturally through Raft configuration.
Mode 3 requires a **placement driver** that intentionally distributes Region
leaders across regions — achievable but a higher-level concern.

### 2d. True multi-master (accept conflicting writes, reconcile later)

**No.** PedraDB does not support multi-master / last-writer-wins / CRDT-style
conflict resolution. This would break strict serializability.

Systems that do this (Cassandra, DynamoDB, ScyllaDB with tunable consistency)
trade correctness for availability. They use vector clocks or timestamps to
detect conflicts and either pick a winner (last-writer-wins) or expose conflicts
to the application. PedraDB's design philosophy (following FDB) is that
correctness is non-negotiable.

---

## Nuance 3: Read consistency levels

Even with strict serializability as the default, distributed systems often
offer weaker read levels for performance:

| Read level | What it means | Latency | PedraDB? |
|-----------|---------------|---------|----------|
| **Strong / linearizable** | Read sees all committed writes up to now | High (Raft ReadIndex or leader lease) | ✅ default |
| **Read-your-writes** | Client sees its own previous writes | Medium (session stickiness) | ✅ automatic via timestamp caching |
| **Bounded staleness** | Read is at most T seconds stale | Low (follower read with timestamp bound) | Future option |
| **Eventual** | Read may be arbitrarily stale | Lowest (any follower) | ❌ not offered |

PedraDB's default is strong reads. Follower reads (for analytics, dashboards,
etc.) could be offered as a configurable opt-in with bounded staleness.

---

## Nuance 4: How Raft interacts with PedraDB's LSM engine

```
Write path (distributed):

  Client → Region Leader → ┌──────────────────┐
                            │  Raft Log Entry   │  (consensus: replicate to followers)
                            └────────┬─────────┘
                                     │ apply
                            ┌────────┴─────────┐
                            │  PedraDB          │  (embedded engine)
                            │  WAL → MemTable   │  (local durability + indexing)
                            │  → SST → Compact  │
                            └──────────────────┘
```

Key design decisions:

1. **Raft log is NOT the same as PedraDB's WAL.** The Raft log is the consensus
   log (replicated). PedraDB's WAL is the local durability log. Both exist:
   - Raft log: used for replication and recovery across nodes.
   - PedraDB WAL: used for local crash recovery (if the node reboots).
   
   In practice, the Raft log can *replace* the WAL for committed entries —
   the Raft log is already replicated and durable. But PedraDB's WAL may be
   useful for uncommitted entries (local transactions not yet replicated).

   TiKV uses two separate RocksDB instances (one for data, one for Raft logs).
   PedraDB would similarly maintain a separation.

2. **Raft apply is sequential per Region.** Each Raft log entry is applied
   to PedraDB in order. This is a single-writer per Region — PedraDB's
   transaction manager handles this naturally (one active write transaction
   at a time per Region).

3. **Compaction is local.** PedraDB's Lazy Leveling compaction runs on each
   node independently. It does not coordinate across nodes. Each replica may
   compact at different times, but this is fine — the Raft log defines the
   canonical data order, and compaction is an internal optimization.

---

## Nuance 5: Distributed transactions across Regions

When a transaction touches keys in multiple Regions (potentially on different
nodes), PedraDB needs distributed transaction coordination. Two approaches:

### Approach 1: Percolator (TiKV's model)

```
Transaction writes keys K1 (Region A), K2 (Region B):

Phase 1 — PREWRITE:
  ┌─ Lock K1 on Region A (leader) ──── primary lock ─┐
  │  Lock K2 on Region B (leader) ──── secondary     │
  └───────────────────────────────────────────────────┘

Phase 2 — COMMIT:
  ┌─ Commit K1 (release primary lock) ─┐
  │  Commit K2 (release secondary) ────│  (async)
  └─────────────────────────────────────┘
```

- **Primary lock** is the transaction's anchor. If the coordinator dies, any
  node encountering a secondary lock can check the primary: if committed,
  the whole transaction committed; if rolled back, it aborted.
- **Timestamp oracle** (a single counter served via Raft) provides
  monotonically increasing timestamps for snapshot isolation.
- **Lock cleanup:** if a transaction is interrupted, locks are cleaned up
  lazily when another transaction encounters them (lock stealing).

### Approach 2: Timestamp Ordering with HLC (CockroachDB's model)

- **Hybrid Logical Clocks (HLC):** each node maintains a clock that combines
  physical time with a logical counter. Provides causally-consistent timestamps
  without a central oracle.
- **Range leases:** each Range (Region) has a leaseholder that serves reads
  and proposes writes. The leaseholder uses HLC timestamps to order
  transactions.
- **Deadlock detection:** transactions detect deadlocks via a global
  transaction registry and abort one side.

### What PedraDB should use

**Percolator (Approach 1)** is simpler to implement and well-documented. It
requires a timestamp oracle (a simple Raft-replicated counter). CockroachDB's
HLC approach is more sophisticated but adds complexity.

However, PedraDB has a unique advantage: **the embedded engine already has
transactions.** The distributed transaction layer can delegate conflict
detection to PedraDB's local OCC, reducing the distributed coordination needed:

1. The coordinator sends PREWRITE to each Region.
2. Each Region's PedraDB instance applies the prewrite using its local
   transaction manager (which already does conflict detection via OCC).
3. If any Region's conflict check fails, the whole transaction aborts.
4. If all succeed, the coordinator commits.

This is simpler than building a separate distributed conflict resolver.

---

## Nuance 6: Sharding — how data is split across nodes

### Range-based sharding (recommended)

PedraDB uses **ordered keys**, so range-based sharding is natural:

- Key space is divided into contiguous **Regions** (e.g., `["", "fff")`,
  `["fff", "ppp")`, `["ppp", "")`)
- Each Region is a contiguous range, stored on one node (replicated via Raft)
- **Splits:** when a Region exceeds a size threshold (e.g., 256 MB), it splits
  into two Regions at a boundary key
- **Merges:** when adjacent Regions are too small, they merge
- **Rebalancing:** the placement driver moves Region replicas between nodes to
  balance load and disk usage

**Advantage:** range scans are efficient (a scan touches few Regions). Hot keys
can be isolated by splitting around them.

**Disadvantage:** hot keys cause hot Regions (mitigated by splitting).

### Hash-based sharding (not recommended)

Keys are hashed to determine placement. Distributes load evenly but destroys
ordering — range scans become scatter-gather across all nodes. This defeats
the purpose of an ordered KV store.

---

## Nuance 7: The CAP theorem and PedraDB

PedraDB chooses **CP** (Consistency over Availability):

| Scenario | What happens |
|----------|-------------|
| Node crash | Raft elects new leader for affected Regions; cluster stays up |
| Network partition (minority side) | Minority side cannot reach Raft majority → rejects writes |
| Network partition (majority side) | Majority side continues normally |
| Network partition (split-brain) | Only the majority side accepts writes; minority side is read-only or unavailable |
| Datacenter failure | If replicas span DCs, the surviving DCs form a majority and continue |

This is the same choice FoundationDB and CockroachDB make. It means PedraDB is
**not eventually consistent** — it does not accept writes that might conflict.
During a partition, the minority side is unavailable, not divergent.

As FDB's documentation states: *"During a network partition FoundationDB
chooses Consistency over Availability. This does not mean that the database
becomes unavailable for clients."* — clients that can reach the majority
continue normally.

---

## Nuance 8: What stays the same (embedded → distributed)

These things do NOT change when PedraDB goes from embedded to distributed:

| Property | Embedded | Distributed |
|----------|----------|-------------|
| API contract | `get`, `put`, `delete`, `range_read`, `begin`, `commit`, `abort` | Same |
| ACID guarantees | Atomic, Consistent, Isolated, Durable | Same |
| Snapshot isolation | Yes (MVCC + local seqnum) | Yes (MVCC + distributed timestamp) |
| Conflict detection | OCC (local, in-memory) | OCC (local per Region) + 2PC coordination |
| LSM optimizations | WiscKey + Monkey + Dostoevsky | Same (each node runs the same engine) |
| Value size | Unlimited (WiscKey value log) | Same (but large values increase replication cost) |
| Transaction semantics | Same | Same |

The **only** things that change:

| Property | Embedded | Distributed |
|----------|----------|-------------|
| Latency | Microseconds (function call) | Milliseconds (network round-trip) |
| Throughput | Single-node limit | Scales with nodes |
| Failure model | Process crash → local recovery | Node crash → Raft leader election |
| Transaction size | Unlimited (local memory) | Practical limit from network serialization |
| Value size | Unlimited (local disk) | Practical limit from replication bandwidth |

---

## Nuance 9: Things that become harder when distributed

1. **Long-running transactions.** A transaction holding locks across multiple
   nodes blocks other transactions. The distributed coordinator must implement
   deadlock detection (wait-die, wound-wait, or timeout-based abort).

2. **Hot keys.** All writes to a hot key go to one Raft leader. This is a
   single point of bottleneck. Mitigations: sub-sharding, write batching,
   leader balancing.

3. **Cross-Region transactions.** A transaction touching keys on Regions in
   different datacenters incurs WAN latency for each phase of 2PC. Can be
   50-200ms per phase. Mitigations: co-locate related data, use locality-aware
   transaction routing.

4. **Schema changes / online migrations.** Changing the key encoding (e.g.,
   adding a new index) requires coordinating across all Regions. FDB layers
   handle this with transactional schema migration; PedraDB layers would too.

5. **Backup / restore.** Must be coordinated across nodes. Each node backs up
   its local Regions; restore must maintain consistency across Regions.

---

## The layered distribution architecture

```
┌─────────────────────────────────────────────────────────────┐
│  SQL DB · Document DB · Graph DB · Time-series DB          │  application layers
├─────────────────────────────────────────────────────────────┤
│  Distributed Transaction Coordinator                       │  2PC / Percolator
│  (timestamp oracle · cross-Region commit · lock cleanup)   │
├─────────────────────────────────────────────────────────────┤
│  Multi-Raft + Placement Driver                             │  consensus + sharding
│  (per-Region Raft · leader election · split/merge/rebalance)│
├─────────────────────────────────────────────────────────────┤
│  PedraDB Embedded Core                                     │  the pillar (unchanged)
│  (ordered KV + ACID transactions + LSM engine)             │
├─────────────────────────────────────────────────────────────┤
│  LSM Engine (WiscKey + Monkey + Dostoevsky)               │  storage
└─────────────────────────────────────────────────────────────┘
```

Everything above "PedraDB Embedded Core" is the **distribution layer**. It is
a separate crate (`pedradb-distributed` or similar), not part of the core.
Applications that don't need distribution use PedraDB embedded directly.

---

## Summary: answers to the original questions

| Question | Answer |
|----------|--------|
| **How do we distribute across nodes?** | Multi-Raft: data sharded into Regions, each Region is a Raft group with replicas. A Placement Driver manages Region assignment and rebalancing. |
| **Is it multi-write?** | Yes, across Regions (different keys have leaders on different nodes). No, within a Region (single Raft leader per key range). Not multi-master for the same key. |
| **Eventually consistent?** | **No.** PedraDB provides strict serializability — the strongest consistency model. It chooses CP (consistency over availability) during partitions. |
| **What consistency level?** | Strict serializable (linearizable reads + serializable isolation). Same as FDB and CockroachDB. |
| **What changes from embedded?** | API and semantics are identical. Latency increases (network), throughput scales horizontally, failure model changes (node crash → Raft election). |
| **When do we build this?** | Not now. The embedded core must be solid first. Distribution is a future layer, after Slices 0–7 (embedded engine + transactions + SST + compaction + GC). |

---

## Deep research follow-up

Protocol-level findings (Percolator paper, TiKV optimistic/pessimistic TX,
PD scheduling, TSO format, CockroachDB Parallel Commits + HLC, openraft vs
raft-rs, etcd guarantees) are documented in:

**[`distribution-deep-research.md`](distribution-deep-research.md)**

Key upgrades from that research (not fully detailed above):

1. **Cross-Region commit target = Parallel Commits**, not classic serial 2PC
   (halves consensus latency).
2. **TiDB defaulted to pessimistic TX** (v3.0.8+) because optimistic aborts
   kill OLTP under contention; PedraDB should support both modes eventually.
3. **TSO is 46-bit physical ms + 18-bit logical** (TiDB); HLC is the
   decentralized alternative (CRDB) but needs NTP discipline.
4. **In-memory locks + pipelined locking** are critical for latency but fragile
   under partition — must be configurable.
5. **PD (placement driver)** is a separate service with store/Region heartbeats
   and three operators: AddReplica, RemoveReplica, TransferLeader.
6. **openraft vs raft-rs** both viable; decision deferred to distribution work.

## Sources

| Ref | Source | Fetched |
|-----|--------|---------|
| [FDB-arch] | FoundationDB Architecture — apple.github.io/foundationdb/architecture.html | 2026-08-10 |
| [FDB-cap] | FDB CAP Theorem analysis — apple.github.io/foundationdb/cap-theorem.html | 2026-08-10 |
| [FDB-cons] | FDB Consistency — apple.github.io/foundationdb/consistency.html | 2026-08-10 |
| [TiKV] | TiKV Overview — docs.pingcap.com/tidb/stable/tikv-overview/ | 2026-08-10 |
| [Deep] | `docs/distribution-deep-research.md` — Percolator, Parallel Commits, PD, TSO, Raft libs | 2026-08-10 |
| [Prior] | `docs/distributed-systems-analysis.md` — ScyllaDB, Ceph, CockroachDB analysis | prior session |
