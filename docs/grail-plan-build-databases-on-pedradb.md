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

## 9. Scylla specifically (why it’s the odd one out)

| Scylla | PedraDB grail |
|--------|----------------|
| Multi-master, tunable / often eventual | Strong local ACID; outer CP Raft |
| Shard-per-core Seastar | Threaded library |
| CQL / wide-column | Ordered KV + layers |

**Do not** force PedraDB to be Scylla. If the company needs Scylla-class, that’s a **different** engineering program (or use Scylla). The grail covers **SQLite → Postgres-ish → TiKV/FDB → etcd** far more naturally than Scylla.

---

## 10. Concrete “how we use PedraDB” recipes

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

### Recipe Q — TiDB niche

```
MySQL client --> SQL layer --> Recipe K
```

---

## 11. Decision log (resolve the “are we wrong?” thread)

| Decision | Proposal |
|----------|----------|
| Is local wrong? | **No** — local kernel is the root of the grail |
| Is multi-node in PedraDB? | **No** — separate products |
| Durability | **Default DataSync + optional async + group commit** |
| First competitive target after kernel | Prefer **embed (SQLite-class)** or **coord (etcd-class)**; not Scylla |
| Relation to fjall | Compete on TX-first kernel; if P0 slips, facade-on-fjall or adopt fjall |
| etcd | Product **on top** of PedraDB + Raft, not PedraDB itself |

---

## 12. Near-term plan (actionable)

1. **Lock RFC-0001** with durability = default sync + optional async; single-writer P0; prefixes; interactive TX.  
2. **Ship P0** (TX + crash) — justify use.  
3. **Pick first product above PedraDB** (S or E) in a **separate** RFC — don’t implement etcd API inside pedradb-core.  
4. Keep distribution/etcd/TiKV docs as **upper-rung research**, not kernel scope.  
5. Revisit “wrong approach” only if P0 can’t beat “just use fjall” for that first product.

---

## 13. One paragraph

PedraDB should stay **local**, with **async available and sync the honest default for commit**, plus **group commit** so we don’t become a single-node etcd performance trap. The grail is not PedraDB replacing Postgres/TiKV/Scylla/**etcd** by itself — it’s PedraDB as the **shared kernel** under embed ACID, then (separately) Raft-backed **etcd-class** coord, **TiKV-class** KV, and **TiDB-class** SQL. Scylla-class AP multi-master is out of the main line. Success is measured first by a tiny correct TX kernel people use, then by one real upper product — not by promising every database on day one.
