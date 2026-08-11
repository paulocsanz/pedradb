# Plan: PedraDB as the substrate to build databases (SQLite → etcd class)

**Status:** strategic plan (draft)  
**Updated:** 2026-08-11  
**Depends on:** RFC-0001 (local kernel), positioning (justify use first)

---

## 1. The confusion to clear first

| Fear | Clarification |
|------|----------------|
| “Maybe we shouldn’t be local” | **Local is correct for the engine.** Every system below has a local store on each process/node. Being *only* local is not a dead end — it’s the **bottom layer** of the grail. |
| “Then we can’t replace TiKV/FDB/etcd” | **PedraDB never replaces them alone.** Products **built on** PedraDB (or PedraDB + Raft + layers) do. Same as RocksDB doesn’t replace TiKV; TiKV embeds RocksDB. |
| “fsync always = etcd-slow” | etcd is slow/write-bound because **Raft + majority fsync + single group**. A **local** engine with **async default + optional sync + group commit** is the industry engine pattern (RocksDB/fjall/LevelDB). |

**Grail sentence:**

> PedraDB is the **smallest correct fast local kernel** (ordered KV + multi-key ACID).  
> On top of it, **we** (or others) build the products that compete with SQLite, Postgres, TiDB, TiKV, Scylla, **etcd**, …

---

## 2. What “alternative to X” really means

You never swap “PedraDB binary” for “Postgres binary.” You swap **a stack**:

```
┌─────────────────────────────────────────────────────────┐
│  Product layer (what users think the database is)         │
│  SQL · CQL · etcd API · Mongo wire · …                    │
├─────────────────────────────────────────────────────────┤
│  Distribution (only if needed)                            │
│  multi-Raft · PD · 2PC · gossip · …                       │
├─────────────────────────────────────────────────────────┤
│  PedraDB  ← always local library on each process          │
│  ordered KV + multi-key ACID                              │
└─────────────────────────────────────────────────────────┘
```

| Target product | Competing with | Stack on PedraDB |
|----------------|----------------|------------------|
| **Embed ACID app DB** | SQLite, redb, LiteFS-ish embeds | **PedraDB alone** (or thin SQL layer later) |
| **Server SQL single-node** | Postgres (single node) | SQL layer + PedraDB (still one process or one machine) |
| **Distributed SQL** | TiDB, CockroachDB | SQL layer + multi-Raft + PedraDB per node |
| **Distributed TX KV** | TiKV, FoundationDB | multi-Raft + TX coordinator + PedraDB per node |
| **AP wide-column / high QPS** | Scylla, Cassandra | Different consistency model — PedraDB’s strong TX core is a **poor direct match**; only if we *choose* AP product (usually **don’t** force PedraDB to be Scylla) |
| **Coordination / config** | **etcd**, ZooKeeper, Consul | Small ordered KV + watches/leases **layer** + single Raft (or multi) + PedraDB as log/state machine storage |

**Important:** Scylla is **AP multi-master**. PedraDB is **CP/ACID-shaped**. The grail is **not** “one binary kills Scylla.” It’s “one kernel enables **CP** databases and embeds”; Scylla-class is a **different design branch** (or a separate product that might not use PedraDB).

---

## 3. Local is not wrong — it’s the grail’s root

### Why every “big DB” is still local underneath

| Product | Local thing on each node |
|---------|---------------------------|
| TiKV | RocksDB (+ raft-engine) |
| FDB | Redwood |
| etcd | bbolt |
| CockroachDB | Pebble |
| Scylla | Custom LSM (Seastar) |
| Postgres | Its own storage (one node or replicas) |
| SQLite | Its own file B-tree |

If PedraDB is excellent **local**, it becomes the replaceable heart of **many** products — exactly RocksDB’s power.

### When “only local” would be wrong

Only if the **company goal** is “ship a managed multi-region SQL cloud next quarter” **and** you refuse to use RocksDB/fjall underneath. Then you’d still implement local storage first; you’d just prioritize the outer product sooner. **The engine being local stays true.**

**Decision:** PedraDB repo stays **local library**. Multi-node lives in **other packages/repos** that depend on PedraDB.

---

## 4. Durability model for the grail (async + optional sync)

### What FDB / TiKV do (internally, simplified)

| System | “Commit” means | Local disk |
|--------|----------------|------------|
| **FDB** | Transaction survived **distributed** pipeline (logs + replication policy) | Storage engines flush/sync as part of role durability; not “embed app chooses sync flag” |
| **TiKV** | Mutation in **Raft log on majority** (durable on quorum) | RocksDB apply can lag; Raft log path uses sync when required |
| **RocksDB/fjall alone** | Whatever the **embedder** asked (`sync` / `persist`) | Default often **async** (OS buffer) |

PedraDB is in the **RocksDB/fjall** row until something wraps it in Raft.

### Recommended PedraDB durability API (plan)

Match engine reality **without** lying about ACID:

```text
commit()                     → default policy (choose one, document hard)
commit_with(CommitOptions)   → { durable: bool } or enum
```

| Policy option | Behavior | Use |
|---------------|----------|-----|
| **`Durable::OsBuffer`** (async) | WAL written to kernel; process crash OK; power loss may lose tail | Bulk load, benches, cache-like |
| **`Durable::DataSync`** | `fdatasync` WAL (group-committed when possible) | **Default for “real” ACID apps** (recommended) |
| **`Durable::All`** | Stronger sync (dir fsync if needed) | Paranoid |

**Group commit:** under concurrent `commit()`, many TXs share one fsync → not etcd-heavy.

**Revised lean (vs earlier “always fsync, no options”):**

- Default: **`DataSync`** (honest ACID for power loss on one machine).  
- Always expose **async** for loaders/benches.  
- Implement **group commit** as soon as multi-threaded commits exist.  
- Do **not** default to pure async and hide the risk (RocksDB footgun).

This is “async + optionally sync” **with safe default**, not “async only like a cache.”

---

## 5. Capability ladder: from PedraDB to each target

Build **up**, not sideways. Each rung is a product or crate.

```
Rung 0  PedraDB kernel          ordered KV + multi-key ACID + durability options
Rung 1  Embed kits              docs, patterns: indexes, queues, tenants (prefixes)
Rung 2  pedra-sql-lite          optional single-node SQL (SQLite niche)
Rung 3  pedra-raft / multi-raft distribution + apply into PedraDB
Rung 4  pedra-kv-dist           TiKV/FDB-class TX KV API
Rung 5  pedra-sql-dist          TiDB/CRDB-class
Rung 6  pedra-coord             etcd-class API (watch, lease, election) on Raft+PedraDB
Rung X  NOT primary             Scylla-class AP multi-master (different physics)
```

### Mapping “alternative to …”

| Competitor | Rung that competes | What PedraDB contributes |
|------------|--------------------|---------------------------|
| **SQLite** | 0–2 | Embed ACID; optional tiny SQL layer |
| **Postgres** (single node) | 2 | SQL + richer types/WAL story on one machine (huge work; optional) |
| **TiKV / FDB** | 3–4 | Local state machine + local TX; outer product adds Raft + distributed TX |
| **TiDB / CRDB** | 5 | SQL layer on rung 4 |
| **etcd** | 6 | PedraDB stores state machine bytes; Raft for consensus; **layer** implements watch/lease/revision API |
| **Scylla** | X | Only if we abandon strong multi-key ACID as core identity — **not the grail path** |

---

## 6. What PedraDB must provide so the grail is real

### Must (kernel)

| Capability | Why |
|------------|-----|
| Ordered keys + range | Indexes, queues, etcd-like key layout, SQL row keys |
| Multi-key ACID (local) | Layers; single-Region TX without inventing MVCC |
| Crash recovery | Trust |
| Clear durability options | App vs Raft apply vs bulk load |
| Single-process multi-thread | Normal servers |
| Stable-enough disk format story | Don’t sled |
| Tiny API | Layers compose |

### Should (substrate hooks) — after P0 works

| Capability | Why |
|------------|-----|
| `apply_batch` without OCC abort | Raft/log apply is already ordered |
| Snapshot / seq number export | Backup, followers, debugging |
| Prefix / subspace helpers (library, not CF zoo) | FDB-style layers |
| Bounded resource use | Embed in products |

### Must not (or grail dies)

| Anti-feature in kernel | Why |
|------------------------|-----|
| Built-in multi-node | Wrong layer; freezes product |
| etcd/Postgres wire in core | Surface explosion |
| AP/LWW multi-master | Contradicts ACID pillar |
| 200 knobs | Loses to fjall by being worse fjall |

---

## 7. Plan by horizon

### Horizon A — Justify PedraDB (this repo only)

**Goal:** Someone uses PedraDB instead of ad-hoc files / non-TX map / half of SQLite use cases.

| Step | Outcome |
|------|---------|
| A1 | MemTable + WAL recover + get/put |
| A2 | Multi-key TX + durable commit (default DataSync) + optional async |
| A3 | Crash test + index-layer example (&lt;100 lines) |
| A4 | Group commit if needed for multi-thread commits |

**Success:** “I can build correct local state with a tiny API.”  
**If this fails vs fjall:** consider facade over fjall or stop.

### Horizon B — Real engine

| Step | Outcome |
|------|---------|
| B1 | SST + range + compaction correct |
| B2 | Benches vs fjall/redb (honest: sync vs async apples-to-apples) |
| B3 | Size limits, GC of old versions |

### Horizon C — First “alternative to X” product (separate crate/repo)

Pick **one** first product — do not chase all:

| Option | Competes with | Why first |
|--------|---------------|-----------|
| **C-embed** | SQLite (subset) | No distributed systems; proves layers |
| **C-coord** | **etcd** (subset) | Small API surface; Raft + PedraDB state machine; huge infra value |
| **C-kv** | TiKV (subset) | Harder; needs multi-Raft + distributed TX |

**Recommendation order for grail narrative:**  
1) Kernel (A–B) → 2) **either** embed excellence **or** etcd-class coord → 3) distributed KV → 4) distributed SQL.  
Scylla-class only if strategy changes.

### Horizon D — Distributed TX KV + SQL

- multi-Raft, PD-like, Parallel Commits / Percolator-style  
- PedraDB per node for apply  
- SQL layer last (TiDB pattern)

---

## 8. etcd specifically (you meant etcd, not “etc.”)

### What etcd is

- Distributed **coordination** store: config, elections, leases, watches.  
- Strong consistency, **single Raft group**, bbolt local.  
- Small data, not app OLTP warehouse.

### How PedraDB helps build an etcd alternative

```
etcd-compatible (or better) API layer
    watches · leases · revisions · TX (stm)
           │
    single- or multi-Raft
           │
    PedraDB as state machine storage
    (ordered keys, atomic apply of log entries)
```

| etcd need | PedraDB | Extra product code |
|-----------|---------|-------------------|
| Ordered KV | Yes | Key layout for revisions |
| Atomic multi-key | Yes (apply_batch / TX) | Map Raft entry → batch |
| Durability | DataSync / Raft log sync | Raft fsync policy |
| Watch | No in kernel | Watch hub on apply |
| Lease / TTL | No in kernel | Lease wheel + delete keys |
| Multi-node | No | Raft membership |

**PedraDB does not speak etcd API.** A **pedra-coord** (name TBD) does, the way TiDB speaks MySQL on TiKV.

### Why not “just use etcd”

You might still use etcd. Building on PedraDB makes sense if you want **one kernel** under coord + KV + SQL later (one ops/skill stack) — the grail. That’s a **platform** bet, not a year-1 requirement.

---

## 9. Horizontally scalable “Postgres” (N writers, M regions)

This is **not** PedraDB alone. It is **Rung 5** on the ladder: distributed SQL
on a distributed TX KV that uses PedraDB **per node** — the same shape as
**TiDB → TiKV → RocksDB** or **CockroachDB → Pebble**.

### What “Postgres-like, horizontally scalable” means

| Piece | Meaning |
|-------|---------|
| **Postgres-like** | SQL, transactions, schemas, indexes, client protocol *or* wire-compatible enough to migrate apps |
| **Horizontally scalable** | Add machines → more storage + more query/TX throughput |
| **M regions** | Keyspace (or table data) split into **M** contiguous ranges (Regions/Ranges) |
| **N writers** | **Not** N processes writing the **same** key without coordination. Means **N nodes can accept writes at once**, each as **leader of some regions** |

### N writers, correctly understood

```
Wrong mental model (multi-master same key):
  Node A and Node B both write key K independently → LWW / conflicts
  = Scylla/Cassandra style

Right mental model (multi-Raft / CRDB / TiKV):
  Region 1 keys [a,m)  leader = Node A  → writes to those keys go to A
  Region 2 keys [m,z)  leader = Node B  → writes to those keys go to B
  N writers = N region-leaders (often on different nodes)
  Same key → still exactly one leader
```

So: **N writers globally, 1 writer per key range.**  
That is how you get scale-out write throughput **and** keep strong transactions.

### Stack (Postgres-horizontal on PedraDB)

```
┌──────────────────────────────────────────────────────────────┐
│  SQL clients (Postgres wire optional; or custom SQL API)     │
├──────────────────────────────────────────────────────────────┤
│  SQL layer (stateless × K)                                   │
│  parse · plan · execute · distributed TX coordinator         │
│  (TiDB / Cockroach SQL role)                                 │
├──────────────────────────────────────────────────────────────┤
│  Distributed KV + TX                                         │
│  M Regions · multi-Raft · placement (PD) · 2PC/Parallel Commits│
│  (TiKV / CRDB KV+TX role)                                    │
├──────────────────────────────────────────────────────────────┤
│  On each node: PedraDB library                               │
│  apply Raft batches · local multi-key ACID for one Region    │
└──────────────────────────────────────────────────────────────┘
```

| Layer | Count | Role |
|-------|-------|------|
| SQL frontends | K (stateless, LB) | Scale **query planning/compute** |
| Nodes holding data | N | Scale **storage + region leaders** |
| Regions | M (≫ N typically) | Unit of sharding + Raft group |
| PedraDB instances | 1 per node (or per process) | Local durable state |

### Data placement (tables → keys → regions)

SQL layer encodes rows/indexes as **ordered keys** (TiDB-style `t{table}_r{row}`,
`t{table}_i{index}_…`).  

- Table data spreads across **many regions** as key space grows.  
- Hot table can split into more regions (M increases).  
- Secondary indexes are **more keys** in the same (or related) key space —  
  updated in the **same distributed TX** as the row (needs multi-key TX across
  regions when index and row live in different regions).

### Transactions spanning regions

| Case | Mechanism |
|------|-----------|
| All keys in **one** region | Local PedraDB TX (or apply_batch) on leader only — fast path |
| Keys in **several** regions | Distributed TX (Percolator / Parallel Commits / 2PC) across region leaders |
| Read-only at snapshot | Reads from leaders (or followers with bounded staleness later) |

PedraDB’s local multi-key ACID makes the **single-region** path trivial and
correct. Cross-region is **outer product** work — same tax TiDB/CRDB already pay.

### N writers vs Postgres vanilla

| Vanilla Postgres | Horizontal stack on PedraDB |
|------------------|-----------------------------|
| One primary writer (typical) | Many region leaders = many write entry points |
| Scale-up vertical | Scale-out add nodes → rebalance regions |
| Streaming replicas mostly read | Every region has Raft followers; leader serves writes |
| Extensions / one process | Stateless SQL × K + storage nodes × N |

### What PedraDB does / does not do in this story

| PedraDB | Outer “distributed Postgres” product |
|---------|--------------------------------------|
| Store bytes, local TX, ranges, crash recovery | SQL, optimizer, catalog |
| Fast apply of ordered batches | Multi-Raft, PD, split/merge regions |
| Durability options local | Quorum durability for “SQL commit” |
| No network | gRPC, wire protocol, auth |

### Effort honesty

Building full Postgres-compatible horizontal SQL is **years** (TiDB/CRDB scale
of investment). The grail path is still valid:

1. PedraDB kernel  
2. Distributed KV (TiKV-class)  
3. SQL layer (Postgres- or MySQL-shaped)

You can stop at (2) and still have a huge platform. (3) is optional product.

### Where “N writers” breaks if you get the model wrong

- Allowing two leaders for the same region → split brain.  
- Skipping distributed TX for multi-region SQL updates → broken indexes.  
- Putting SQL in PedraDB core → kernel dies under surface.  
- Expecting Scylla-like “any node writes any key with ONE” → different product.

---

## 10. Scylla specifically (why it’s off the main line)

### Two different physics

| | **Main grail line** (PedraDB → TiKV/FDB/etcd/SQL-CP) | **Scylla / Cassandra line** |
|--|------------------------------------------------------|-----------------------------|
| **Write to same key** | Single leader / single order (Raft or equivalent) | **Multi-master**: any replica can accept write |
| **Conflict** | Prevent or abort (OCC / locks / Raft order) | **Reconcile later** (timestamp LWW, etc.) |
| **Default consistency** | Strong (serializable / SI / linearizable reads) | **Tunable**; often **eventual** at CL=ONE |
| **Multi-key TX** | Core value (local + distributed 2PC) | **Not** general ACID cross-partition; LWT is special-case Paxos per partition |
| **Indexes as layers** | Safe if same TX as data | Hard: data vs index can diverge under concurrent multi-master |
| **Runtime** | Library + optional Raft product | Seastar shard-per-core, gossip, vnode ring |
| **API culture** | KV/SQL ACID | CQL wide-column, denormalize for queries |

### Why Scylla is “out of the main line” (not “bad”)

1. **Contradicts the pillar thesis**  
   PedraDB’s reason to exist is **multi-key ACID + order** so layers (indexes,
   SQL catalogs, etcd revisions) stay correct. Scylla’s strength is **availability
   and throughput** under a model where concurrent writes to the same key are
   allowed and reconciled — the opposite default.

2. **You cannot get Scylla behavior “for free” from PedraDB**  
   Putting PedraDB under a Scylla-like API either:
   - **Forces single-leader per partition** → you built **TiKV-shaped** CQL, not Scylla; or  
   - **Allows multi-master on top of ACID local stores** → you throw away global
     ACID and reimplement LWW/repair — PedraDB’s TX doesn’t buy the Scylla model.

3. **Different operational and hardware culture**  
   Thread-per-core, shared-nothing shards, compaction strategies as product
   surface, tunable CL per query — a full second platform.

4. **Grail already has a horizontal write story without Scylla**  
   **N region leaders** (above) = horizontal write scale **with** strong TX.  
   That’s the CRDB/TiKV answer to “we need more writers,” not multi-master LWW.

### When Scylla *would* be in-scope

| Situation | Approach |
|-----------|----------|
| Company primary workload is AP, LWW, CQL, extreme single-key QPS | **Use Scylla** (or fork that line) — don’t warp PedraDB |
| Want CQL **with** strong TX per partition only | Possible as a **layer** on multi-Raft+PedraDB (more TiKV than Scylla) |
| Marketing “we replace everything including Scylla” | Dishonest unless you build a second engine/product line |

### One sentence

**Main line = CP / ACID / single-writer-per-key-range / layers.**  
**Scylla line = AP / multi-master / tunable / repair.**  
PedraDB is designed for the first; calling Scylla “out of main line” means
**don’t design the kernel for the second**, not “Scylla is irrelevant as a product in the market.”

---

## 11. Concrete “how we use PedraDB” recipes

### Recipe S — SQLite niche (embed)

```
App
 └─ PedraDB TX: put(data), put(index), commit(DataSync)
```

### Recipe E — etcd niche (coord product)

```
Client --gRPC--> Coord API
                   └─ Raft
                        └─ apply → PedraDB.apply_batch
                        └─ notify watches
```

### Recipe K — TiKV niche (distributed KV)

```
Client --> TX coordinator (2PC / Parallel Commits)
             └─ per Region: Raft leader
                    └─ PedraDB (local TX or apply_batch)
```

### Recipe Q — TiDB / MySQL-wire distributed SQL

```
MySQL client --> SQL layer --> Recipe K
```

### Recipe P — Postgres-like horizontally scalable SQL (N writers, M regions)

```
Postgres clients (or PG-wire)
       │
  SQL layer × K (stateless)
       │  encode rows/indexes as ordered keys
       │  distributed TX if multi-region
       ▼
  M Regions (multi-Raft) on N nodes
       │  each Region leader = one writer for that key range
       │  N writers total ≈ leaders spread across nodes
       ▼
  PedraDB on each node (local apply / local TX)
```

See §9 for full semantics of N writers / M regions.

---

## 12. Decision log (resolve the “are we wrong?” thread)

| Decision | Proposal |
|----------|----------|
| Is local wrong? | **No** — local kernel is the root of the grail |
| Is multi-node in PedraDB? | **No** — separate products |
| Durability | **Default DataSync + optional async + group commit** |
| First competitive target after kernel | Prefer **embed (SQLite-class)** or **coord (etcd-class)**; not Scylla |
| Horizontal Postgres | **Rung 5**: SQL + multi-Raft + PedraDB; N writers = N region leaders |
| Scylla | **Off main line** — AP multi-master ≠ ACID pillar; use Scylla or separate program |
| Relation to fjall | Compete on TX-first kernel; if P0 slips, facade-on-fjall or adopt fjall |
| etcd | Product **on top** of PedraDB + Raft, not PedraDB itself |

---

## 13. Near-term plan (actionable)

1. **Lock RFC-0001** with durability = default sync + optional async; single-writer P0; prefixes; interactive TX.  
2. **Ship P0** (TX + crash) — justify use.  
3. **Pick first product above PedraDB** (S or E) in a **separate** RFC — don’t implement etcd API inside pedradb-core.  
4. Keep distribution/etcd/TiKV docs as **upper-rung research**, not kernel scope.  
5. Revisit “wrong approach” only if P0 can’t beat “just use fjall” for that first product.

---

## 14. One paragraph

PedraDB should stay **local**, with **async available and sync the honest default for commit**, plus **group commit** so we don’t become a single-node etcd performance trap. The grail is not PedraDB replacing Postgres/TiKV/Scylla/**etcd** by itself — it’s PedraDB as the **shared kernel** under embed ACID, then (separately) Raft-backed **etcd-class** coord, **TiKV-class** KV, and **horizontal Postgres/TiDB-class SQL** (N writers = many region leaders, M regions, distributed TX when needed). Scylla-class AP multi-master is out of the main line because it allows concurrent writers on the **same** key and reconciles later — incompatible with the ACID/index-layer thesis. Success is measured first by a tiny correct TX kernel people use, then by one real upper product — not by promising every database on day one.
