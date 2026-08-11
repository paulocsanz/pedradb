# TiDB: how it operates (and where it sits vs PedraDB / FDB / TiKV / Scylla)

> TiDB is **not** a storage engine. It is a **distributed MySQL-compatible SQL
> database** whose storage and distributed transactions live in **TiKV**, whose
> metadata brain is **PD**, and whose optional OLAP path is **TiFlash**.
>
> In PedraDB vocabulary: TiDB is a **layer**. TiKV is the (almost) pillar — but
> the pillar is RocksDB without core TX, so TiDB/TiKV had to bolt on Percolator.

Primary sources (PingCAP docs, fetched 2026-08-11):
- [TiDB Architecture](https://docs.pingcap.com/tidb/stable/tidb-architecture/)
- [TiDB Storage](https://docs.pingcap.com/tidb/stable/tidb-storage/)
- [TiDB Computing](https://docs.pingcap.com/tidb/stable/tidb-computing/)
- Prior: TiKV overview, optimistic/pessimistic TX, PD scheduling, TSO
  (`distribution-deep-research.md`)

---

## TL;DR

| Question | TiDB answer |
|----------|-------------|
| What is it? | Distributed HTAP SQL DB, MySQL protocol compatible |
| Where is data? | **TiKV** (row KV) + optional **TiFlash** (columnar replica) |
| Local disk engine? | **RocksDB** inside each TiKV (data + separate RocksDB for Raft log) |
| How distributed? | **Multi-Raft Regions** + **PD** scheduler + **TSO** |
| Multi-write? | Multi-leader across Regions; **single leader per Region/key range** |
| Consistent? | **Strong** (SI / RR); not eventual. CP under partition |
| ACID multi-key TX? | **Yes** — Percolator 2PC on TiKV, coordinated by TiDB |
| vs PedraDB | TiDB ≈ **SQL layer** PedraDB wants others to build; TiKV ≈ **distributed KV** PedraDB could become — but PedraDB wants TX **in the embedded core** first |

---

## The four components

```
                    MySQL clients
                          │
              ┌───────────┴───────────┐
              │   TiDB Server (×N)    │  stateless SQL layer
              │   parse · optimize    │  MySQL protocol
              │   plan · coprocessor  │  NO local data
              └───────────┬───────────┘
                          │ gRPC
         ┌────────────────┼────────────────┐
         │                │                │
    ┌────┴────┐     ┌─────┴─────┐    ┌─────┴─────┐
    │   PD    │     │   TiKV    │    │  TiFlash  │
    │ (brain) │     │ row store │    │ columnar  │
    │ meta +  │     │ multi-    │    │ (OLAP)    │
    │ TSO +   │     │ Raft +    │    │ Raft      │
    │ schedule│     │ RocksDB   │    │ learner   │
    └─────────┘     └───────────┘    └───────────┘
```

### 1. TiDB Server — SQL layer (stateless)

From architecture docs:

- Exposes **MySQL protocol** endpoint.
- Parses SQL, optimizes, builds a **distributed execution plan**.
- **Does not store user data.** Horizontally scalable behind LVS / HAProxy /
  TiProxy / etc.
- Turns SQL into TiKV (and TiFlash) API calls.
- Runs the **transaction coordinator** (optimistic or pessimistic mode;
  default pessimistic since v3.0.8).

This is exactly the FDB/PedraDB **layer** idea — except TiDB is the product,
and the layer is fixed to SQL/MySQL, not “any data model.”

### 2. PD (Placement Driver) — cluster brain

- Stores **Region → node** metadata and cluster topology.
- **TSO:** allocates globally unique, monotonic timestamps (46-bit physical ms
  + 18-bit logical) for MVCC and 2PC.
- **Scheduler:** rebalances Regions (AddReplica / RemoveReplica /
  TransferLeader), handles store Up/Down/Offline/Tombstone.
- Deployed as an odd-sized Raft-backed cluster (typically ≥3).

Without PD there is no TiDB cluster. It is not optional glue.

### 3. TiKV — distributed transactional KV (the real store)

- Ordered **Key-Value map** (seek + next).
- Data split into **Regions** `[StartKey, EndKey)` — default ~**256 MiB**.
- Each Region is a **Raft group** (default 3 replicas); **leader** serves
  reads/writes for that range.
- Local persistence: **RocksDB** (they explicitly say building a standalone
  engine is too hard — so they wrap RocksDB).
- Separate RocksDB often used for **Raft logs** vs data (I/O separation).
- **Native distributed TX at KV level** (Percolator) — this is how TiDB gets
  SQL ACID.

### 4. TiFlash — columnar extension (optional HTAP)

- Special storage nodes that keep a **columnar** replica of data.
- Accelerate analytical queries without a second ETL pipeline.
- Learner-style replication path (does not take the same write leadership role
  as TiKV leaders for OLTP).

TiDB markets **HTAP**: OLTP on TiKV + OLAP on TiFlash, one SQL surface.

---

## How SQL becomes Key-Value (the layer mapping)

This is the heart of “TiDB as a layer on TiKV.” From computing docs:

### Row data

```
Key:   t{TableID}_r{RowID}
Value: [col1, col2, col3, ...]
```

- `TableID` unique cluster-wide.
- `RowID`: if integer PK, PK value is used as RowID (optimization).
- All rows of a table share prefix `t{TableID}_r` → **range scan = table scan**.

### Unique / primary index

```
Key:   t{TableID}_i{IndexID}_{indexedColumnsValue}
Value: RowID
```

### Non-unique secondary index

```
Key:   t{TableID}_i{IndexID}_{indexedColumnsValue}_{RowID}
Value: null
```

### Constants

```
tablePrefix     = 't'
recordPrefixSep = 'r'
indexPrefixSep  = 'i'
```

### Example

```sql
CREATE TABLE User (
  ID int PRIMARY KEY,
  Name varchar(20),
  Role varchar(20),
  Age int,
  KEY idxAge (Age)
);
-- TableID=10, rows (1,TiDB,...), (2,TiKV,...), (3,PD,...)
```

```
t10_r1 --> ["TiDB", "SQL Layer", 10]
t10_r2 --> ["TiKV", "KV Engine", 20]
t10_r3 --> ["PD", "Manager", 30]

t10_i1_10_1 --> null   -- idxAge
t10_i1_20_2 --> null
t10_i1_30_3 --> null
```

**Why this matters for PedraDB:** this is the textbook FDB layer pattern —
encode tables + indexes as ordered KV, update them in **one transaction**.
TiDB can only do that because TiKV provides multi-key TX. RocksDB alone would
not be enough.

Metadata (schema) is also stored in TiKV under `m_` prefixes, with a global
schema version key for DDL coordination.

---

## Transaction path (SQL → distributed TX)

```
Client BEGIN
  → TiDB gets start_ts from PD (TSO)
  → statements buffer writes / take pessimistic locks (default mode)
COMMIT
  → TiDB runs Percolator 2PC against TiKV Regions:
       prewrite (locks + data) across Regions
       commit_ts from PD
       commit primary, async secondary cleanup
```

Details already in `distribution-deep-research.md`:

| Mode | When | Behavior |
|------|------|----------|
| **Pessimistic** (default ≥3.0.8) | Contended OLTP | Locks on DML; MySQL-like RR; fewer commit aborts |
| **Optimistic** | Low contention | Classic Percolator; conflict at commit; app must retry |

Isolation: **Repeatable Read** (default, MySQL-compatible) or Read Committed.
Not full strict serializable like FDB/Cockroach SERIALIZABLE default — closer
to SI/RR with MySQL quirks (e.g. no InnoDB gap locking).

---

## Multi-write, consistency, CAP

| Property | TiDB/TiKV |
|----------|-----------|
| Multi-write different keys on different nodes | **Yes** — different Region leaders |
| Multi-write same key on two nodes | **No** — one Raft leader per Region |
| Eventual consistency? | **No** (default strong path) |
| CAP | **CP** — Raft majority required |
| Cross-Region ACID | **Yes** (2PC) |
| vs Scylla multi-master LWW | Completely different model |

---

## TiDB vs the other systems (one matrix)

| | **TiDB** | **TiKV alone** | **FDB** | **CockroachDB** | **Scylla** | **PedraDB** |
|--|----------|----------------|---------|-----------------|------------|-------------|
| Product surface | MySQL SQL | Ordered TX KV | Ordered TX KV | PostgreSQL SQL | CQL / DynamoDB | Ordered TX KV (pillar) |
| Layer vs engine | SQL **layer** on TiKV | Distributed engine | Full distributed DB | SQL + Pebble + Raft | Full AP NoSQL | Embedded engine first |
| Local store | RocksDB (in TiKV) | RocksDB | Redwood B-tree | Pebble | Custom LSM+Seastar | Own LSM |
| TX in local engine? | No (in TiKV TX layer) | Bolted on RocksDB | System-wide | Bolted on Pebble | LWT only | **Yes, core** |
| Distribution | Multi-Raft | Multi-Raft | Decoupled roles | Multi-Raft | Token ring + gossip | Future multi-Raft |
| Consistency | Strong SI/RR | Strong SI | Strict serializable | Serializable | Tunable / often eventual | Strict serializable target |
| HTAP | TiFlash columnar | — | Layers | — | — | Out of core |

---

## Where TiDB sits in the four families

```
Family 1: Multi-Raft + strong TX
          TiKV ──────────── storage + TX
          TiDB ──────────── SQL layer on top of Family 1
          CockroachDB ───── SQL + storage fused as one product

Family 2: Decoupled roles + strong TX     FoundationDB
Family 3: Embedded TX pillar              PedraDB
Family 4: Dynamo/Cassandra AP             ScyllaDB
```

**TiDB is a Family-1 SQL product.**  
**TiKV is Family-1 storage.**  
Together they implement what FDB would call a **SQL layer on a transactional KV**
— but their KV did not start with TX in a clean embedded core; they wrapped
RocksDB and invented the TX layer (years of work).

---

## What PedraDB should learn from TiDB

### Copy the idea

1. **SQL (or any model) as a layer over ordered TX KV** — TiDB’s
   `t…_r…` / `t…_i…` encoding is the canonical example.
2. **Indexes updated in the same transaction as rows** — only possible with
   multi-key ACID.
3. **Stateless compute, stateful storage** — scale SQL frontends independently
   of storage (same as FDB clients vs storage servers).
4. **PD-like control plane** when distributed: metadata, TSO, scheduling.
5. **Coprocessor pushdown** — TiKV runs part of the computation near data
   (filters/aggregates per Region). Layers on PedraDB may want a similar hook.

### Do not copy blindly

1. **Wrapping RocksDB without core TX** — PedraDB’s thesis is the opposite:
   TX belongs in the embedded pillar.
2. **MySQL compatibility as core identity** — PedraDB core stays model-free;
   a future `pedradb-mysql` layer is optional, not the core.
3. **Fusing product and layer** — TiDB the company sells SQL; PedraDB sells
   the pillar. Multiple layers (SQL, document, graph) should remain possible.
4. **RocksDB’s write-amp and uniform Bloom** — PedraDB adopts WiscKey, Monkey,
   Lazy Leveling instead.

---

## PedraDB mapping (how TiDB would look on PedraDB)

```
Today (PingCAP):
  MySQL app → TiDB Server → TiKV (Percolator + multi-Raft + RocksDB) + PD

PedraDB analogue (future):
  MySQL app → pedradb-sql layer → pedradb-distributed (multi-Raft + Parallel Commits)
                              → pedradb-core (embedded TX + LSM)
                              → pedradb-pd (TSO + schedule)
```

Difference that matters:

| Concern | TiDB/TiKV | PedraDB path |
|---------|-----------|--------------|
| Single-node embed | No (always cluster) | **Yes** — core alone |
| Local ACID without Raft | No | **Yes** |
| Distributed TX complexity | Built because RocksDB has no TX | Extends existing local TX |
| Engine optimizations | Inherit RocksDB limits (+ Titan opt) | WiscKey + Monkey + Dostoevsky from day 1 |

**TiDB validates PedraDB’s layer vision in production at massive scale.**  
**TiDB also shows the tax of building TX after the fact on RocksDB** — the tax
PedraDB refuses to pay.

---

## Honest “does PedraDB replace TiDB?”

| Goal | Answer |
|------|--------|
| Drop-in MySQL-compatible distributed SQL today | **No** — TiDB is a mature product; PedraDB has no SQL layer yet |
| Better local/embedded transactional KV under a future SQL layer | **Yes, that’s the bet** |
| Replace TiKV under TiDB | Theoretically a long-term research idea (swap RocksDB path); not a near-term goal |
| Compete with TiDB Cloud as a company product | Different scope; PedraDB is infrastructure pillar first |

---

## Sources

| Ref | Source |
|-----|--------|
| [Arch] | docs.pingcap.com/tidb/stable/tidb-architecture/ |
| [Storage] | docs.pingcap.com/tidb/stable/tidb-storage/ |
| [Compute] | docs.pingcap.com/tidb/stable/tidb-computing/ |
| [Deep] | `docs/distribution-deep-research.md` (Percolator, PD, TSO, pessimistic TX) |
| [TiKV] | docs.pingcap.com TiKV overview |
