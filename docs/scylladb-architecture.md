# ScyllaDB: how it operates (and where it sits vs PedraDB / FDB / TiKV)

> ScyllaDB is a high-throughput NoSQL database: Cassandra-compatible (CQL) and
> optionally DynamoDB-compatible (Alternator). It is **not** in the same family
> as FoundationDB, TiKV, CockroachDB, or PedraDB. It optimizes for a different
> contract: **tunable availability + partition tolerance**, multi-master writes,
> and extreme single-node efficiency — **not** strict serializable multi-key
> transactions by default.

Sources: ScyllaDB GitHub README, Seastar README, Scylla consistency-level docs,
Scylla shared-nothing glossary (fetched 2026-08-10). Prior notes in
`distributed-systems-analysis.md`.

---

## TL;DR

| Question | ScyllaDB answer |
|----------|-----------------|
| What is it? | Distributed wide-column / Dynamo-style NoSQL (Cassandra + DynamoDB APIs) |
| Local storage? | **Custom LSM** (not RocksDB), written in C++ on Seastar |
| How distributed? | **Token ring** + Gossip + vnodes (Cassandra model) |
| Multi-write? | **Yes — multi-master.** Any replica can accept a write |
| Consistent? | **Tunable.** Default path is often eventual; QUORUM is common; LWT is special-case linearizable |
| ACID multi-key TX? | **No** (only Lightweight Transactions on single partition / compare-and-set) |
| CAP default | **AP-leaning** (prefer availability; consistency is a per-query knob) |
| PedraDB replace it? | **No as a whole.** Different product category. PedraDB could theoretically back a different stack, not Scylla's Seastar LSM |

---

## Architecture in layers

```
┌─────────────────────────────────────────────────────────────┐
│  Client (CQL / Alternator DynamoDB API)                     │
├─────────────────────────────────────────────────────────────┤
│  Coordinator node (any node can coordinate a request)       │
│  - Pick replicas for partition key via token ring           │
│  - Enforce consistency level (ONE, QUORUM, ALL, …)          │
├─────────────────────────────────────────────────────────────┤
│  Replication: RF copies on nodes determined by token +      │
│  snitch (topology-aware: rack/DC)                           │
├─────────────────────────────────────────────────────────────┤
│  Per-node: Seastar shard-per-core                           │
│  - 1 reactor thread per CPU core                            │
│  - Each shard owns a slice of tokens + its own memtables,   │
│    SSTables, cache, I/O queues — NO shared locks            │
├─────────────────────────────────────────────────────────────┤
│  Local storage: custom LSM (memtable → SSTable → compaction)│
│  NOT RocksDB. Deeply fused with Seastar futures/I/O         │
└─────────────────────────────────────────────────────────────┘
```

### Layer 1 — Local storage (custom LSM)

- Written from scratch in **C++**, not RocksDB.
- Classic LSM ideas: memtable, commit log / WAL, SSTables, compaction.
- Compaction strategies are Cassandra-heritage (Size-Tiered, Leveled, Time-Window, Incremental, etc.) — operational knobs, not Dostoevsky Lazy Leveling.
- **Deeply coupled to Seastar.** You cannot swap the storage engine without rewriting the database. This is the opposite of TiKV/CockroachDB (pluggable/wrapped engine) and of PedraDB's "engine as pillar" model.

### Layer 2 — Distribution (Cassandra / Dynamo model)

- **Token ring:** partition key → hash (Murmur3) → token → primary owner + RF-1 replicas walking the ring (with vnodes for balance).
- **Gossip:** nodes exchange membership and state peer-to-peer. No central Placement Driver like TiKV's PD, no FDB cluster controller for data plane.
- **Any node can be coordinator** for a client request. Coordinator forwards to the replicas that own that partition's token range.
- **Multi-datacenter:** snitch + NetworkTopologyStrategy place replicas per DC; consistency levels like `LOCAL_QUORUM` / `EACH_QUORUM` control cross-DC behavior.

### Layer 0 — Runtime (Seastar: the real secret sauce)

From Scylla's shared-nothing glossary and Seastar docs:

- **Shard-per-core / shared-nothing:** one application thread per CPU core.
- Each core has its own memory, queues, and data shard.
- **No cross-core locking.** Cross-core work is explicit message passing.
- Custom CPU and I/O schedulers to keep latencies predictable under load.
- This is why Scylla is often **2–10× Cassandra** on the same hardware: Cassandra is JVM + shared structures + locks; Scylla is close-to-metal async C++.

---

## Write path (normal path — not LWT)

```
Client
  → any coordinator
  → compute token from partition key
  → send write to RF replicas (async fan-out)
  → wait until consistency level is satisfied
       e.g. ONE  = 1 replica ack
            QUORUM = majority of RF
            ALL = every replica
  → return success to client
  → remaining replicas catch up (hinted handoff, repair, read repair)
```

**Critical property:** under `ONE` or even `QUORUM`, different clients can write
the **same key on different replicas concurrently**. Conflicts are resolved by
**timestamps** (last-write-wins / client timestamp), not by a global transaction
manager. That is **true multi-master multi-write**.

### Read path

```
Client → coordinator → query enough replicas for CL
  → merge versions (timestamp) → return newest
  → optionally read-repair (push newer to lagging replicas)
```

With `CL=ONE`, a read can miss a write that another replica has (stale read).
With `CL=QUORUM` and writes also at `QUORUM`, you get the classic Cassandra
**R + W > RF** rule for *per-key* strong-ish consistency (still not multi-key
serializability).

---

## Consistency model (tunable — the opposite of FDB)

From Scylla CQL consistency docs:

| Level | Who must respond | Typical use |
|-------|------------------|-------------|
| `ANY` | Any node (even hinted handoff) | Never lose a write; weakest |
| `ONE` / `LOCAL_ONE` | One replica | Lowest latency |
| `QUORUM` / `LOCAL_QUORUM` | Majority (cluster / local DC) | Common production default |
| `ALL` | Every replica | Strongest, lowest availability |
| `EACH_QUORUM` | Majority in **each** DC | Multi-DC writes |
| `SERIAL` / `LOCAL_SERIAL` | For **LWT** reads | Linearizable vs in-flight LWT |

**Default product philosophy:** AP with tunable C. You choose per-query.
Scylla (and Cassandra) stay available on both sides of a partition if you
pick low CL — at the cost of possible divergence.

**FoundationDB / PedraDB / TiKV / CRDB:** CP. Minority side stops taking
divergent writes. Consistency is not a per-query dial for multi-key ACID.

### Lightweight Transactions (LWT) — the exception

- Paxos-based **compare-and-set** on a **single partition**.
- Enables `IF NOT EXISTS` / conditional updates.
- Uses `SERIAL` / `LOCAL_SERIAL` for linearizable semantics **around that partition**.
- **Not** general multi-partition ACID transactions.
- Higher latency than normal writes (consensus round).

So Scylla can do "linearizable single-row conditional write" — it cannot do
"transfer money across two partitions in one ACID transaction" the way FDB/PedraDB can.

---

## Multi-write: yes, really

| Meaning | Scylla | FDB | TiKV/CRDB | PedraDB distributed |
|---------|--------|-----|-----------|---------------------|
| Multiple nodes accept writes for **different** keys | ✅ | ✅ (via pipeline) | ✅ multi-Region leaders | ✅ multi-Region leaders |
| Multiple nodes accept writes for the **same** key | ✅ **multi-master** | ❌ | ❌ single leader/leaseholder | ❌ single leader |
| Conflict resolution | Timestamp / LWW | Abort (OCC) | Abort / lock | Abort (OCC) |
| Cross-partition ACID | ❌ (LWT only single partition) | ✅ | ✅ (2PC) | ✅ (2PC) |

Scylla is the clearest example of **multi-master multi-write** in this survey.
That is exactly the model PedraDB **rejects** as default, because layers cannot
build correct secondary indexes on top of LWW races.

---

## Data model (why it feels different)

- **Wide-column** (Cassandra): keyspace → table → partition key → clustering columns → cells.
- Access is designed around **partition key locality**. Multi-partition queries are expensive / denormalized away.
- No secondary indexes as a first-class "always consistent with TX" feature the way an FDB layer would implement them.
- Modeling rule of thumb: **denormalize for your queries** (Cassandra way), don't join.

PedraDB's model is the opposite foundation: **normalized KV + transactions**, indexes as layers.

---

## Where Scylla sits among the four families

```
Family 1: Multi-Raft + strong TX     TiKV, CockroachDB
Family 2: Decoupled roles + strong TX FoundationDB
Family 3: Embedded TX pillar          PedraDB
Family 4: Dynamo/Cassandra AP NoSQL   ScyllaDB, Cassandra, DynamoDB
```

Scylla is **Family 4**.

| Dimension | ScyllaDB (Family 4) | PedraDB (Family 3→1) |
|-----------|---------------------|----------------------|
| Goal | Millions of ops, p99 low latency, always-on | Correct foundation for building databases |
| Consistency | Tunable / often eventual | Strict serializable |
| Multi-write same key | Yes (LWW) | No |
| Multi-key ACID | No (LWT single partition only) | Yes |
| Engine | Custom LSM + Seastar | Clean-room LSM + academic opts |
| Swap engine? | Impossible without rewrite | Distribution is a layer; core is embeddable |
| Language | C++23 + Seastar | Rust, forbid(unsafe) |
| Testing culture | Heavy functional/perf; not FDB-style full-system sim as the identity | Aim FDB-style deterministic sim |

---

## What PedraDB should (and should not) learn from Scylla

### Worth studying

1. **Shard-per-core / shared-nothing efficiency** — when PedraDB is embedded in a high-QPS process, avoiding cross-core contention matters. Seastar is extreme; Rust async + careful ownership is the softer analogue.
2. **Operational compaction knobs** — Scylla/Cassandra have decades of war stories on compaction storms; informs Lazy Leveling pacing design.
3. **Topology-aware replica placement** — snitch/rack/DC awareness is table stakes for multi-DC later.
4. **Predictable latency under load** — custom I/O and CPU scheduling; PedraDB's "no artificial write throttling" (Pebble lesson) rhymes with Scylla's scheduler philosophy.

### Not to copy

1. **Eventual consistency as the core contract** — breaks the "pillar for other databases" mission.
2. **LWW multi-master** — layers cannot maintain correct indexes.
3. **Seastar-fused storage** — makes the engine non-reusable as a library pillar.
4. **Wide-column denormalization as the only modeling path** — FDB layers need multi-key TX.

---

## Honest "can PedraDB replace Scylla?"

| Workload | Replace Scylla with PedraDB? |
|----------|------------------------------|
| Massive write QPS, single-partition access, can tolerate tunable consistency | **No** — Scylla is purpose-built; PedraDB distributed multi-Raft will not match Seastar throughput soon |
| Need multi-key ACID, secondary indexes as layers, strict serializability | **Yes directionally** — that's PedraDB's niche; Scylla is the wrong tool |
| DynamoDB-compatible API at scale | Scylla Alternator wins; PedraDB would need a full Alternator-like layer + distribution |
| Embedded library inside another process | **PedraDB yes; Scylla no** (Scylla is a server) |

**Bottom line:** Scylla is a **sibling competitor to Cassandra/DynamoDB**, not to FoundationDB. PedraDB competes with **RocksDB + FDB's API idea**, and later with **TiKV's distribution shape** — not with Scylla's AP multi-master NoSQL niche.

---

## Sources

| Ref | Source |
|-----|--------|
| [Scylla-GH] | github.com/scylladb/scylladb README |
| [Seastar] | github.com/scylladb/seastar README |
| [CL] | docs.scylladb.com — CQL Consistency Levels |
| [SN] | scylladb.com glossary — Shared Nothing Architecture |
| [Prior] | `docs/distributed-systems-analysis.md` Scylla section |
