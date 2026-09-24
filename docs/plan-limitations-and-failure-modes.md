# Plan limitations: where we might discover it doesn’t work later

**Status:** adversarial review of the grail plan  
**Updated:** 2026-08-11  
**Complements:** `upsides-only.md` (positive case), `grail-plan-…`, RFC-0001,
[`performance-ceiling-option-preservation-and-sled-layer.md`](performance-ceiling-option-preservation-and-sled-layer.md)
(ceilings vs Rocks/Pebble/sled/redwood, anti-corner checklist, sled API layer, B-tree-class reads without dual store).

Honest constraints of:

> tiny local ACID ordered KV → API layers → optional multi-Raft → SQL / DCS / …

Not “reasons to quit tomorrow” — **places to watch** so we don’t paint ourselves into a corner.

---

## 1. Performance

### 1.1 Local fsync vs “blazing embed”

| Risk | Why it shows up later |
|------|------------------------|
| **Commit latency floor** | Strong durable commit ≈ one `fdatasync` (or group). Sub-ms p99 for *every* small TX is hard on mediocre disks/cloud volumes. |
| **Losing benches to fjall/RocksDB defaults** | They default async; if we default sync, marketing/benches look worse unless we document apples-to-apples. |
| **Group commit needs concurrency** | Single-threaded commit storm still pays 1 fsync per TX. |

**Would hurt:** trading apps wanting &lt;100µs durable commit on commodity cloud disks.  
**Mitigation:** group commit; optional async; good disk guidance; don’t promise Redis latency.

### 1.2 LSM write amp / compaction

| Risk | Why |
|------|-----|
| **Write amplification** | Classic LSM can rewrite data many times; large values make it worse until value log exists. |
| **Compaction storms** | Under sustained write + range deletes, latency spikes (RocksDB lore). |
| **Research opts late** | Lazy Leveling / Monkey only help if implemented correctly; until then we’re “another LSM.” |

**Would hurt:** heavy update-in-place large docs, wide tables rewritten often.  
**Mitigation:** value threshold log (P2); simple but solid compaction first; measure early.

### 1.3 Read amp / many levels

| Risk | Why |
|------|-----|
| **Point lookup cost** grows with levels/files without good Bloom allocation. |
| **Range merge** of many iterators more expensive than B-tree leaf walk (redb/LMDB win some read-heavy embeds). |

**Would hurt:** read-heavy embed with cold cache vs redb/LMDB.  
**Mitigation:** caches, Bloom policy, don’t pick LSM if primary workload is pure read-heavy small DB.

### 1.4 Single-writer TX (if we keep it)

| Risk | Why |
|------|-----|
| **One write TX at a time** caps multi-core write concurrency inside **one process**. |
| Disjoint-key parallel writes serialize at commit. |

**Would hurt:** single fat node, many cores, high contention-free write QPS all through one PedraDB.  
**Mitigation:** OCC multi-writer (P1); or shard **above** PedraDB (multiple Db instances / regions) — which is the distributed product anyway.

### 1.5 Cross-region distributed TX

| Risk | Why |
|------|-----|
| **2PC / Parallel Commits latency** | Multi-region SQL TX = multiple Raft RTTs (WAN worse). |
| **Abort rate** under hot keys / long TX (OCC). |
| **Chatty SQL** (ORM N+1) × distributed TX = death. |

**Would hurt:** chatty multi-table TX spanning the whole keyspace on multi-region geo.  
**Mitigation:** locality-aware keys; single-region fast path; statement design; pessimistic later if needed (big cost).

### 1.6 Horizontal SQL / “Postgres scale-out”

| Risk | Why |
|------|-----|
| **Not free** | Compatibility + optimizer + dist TX = years (TiDB/CRDB scale). |
| **Secondary indexes** often force multi-region TX. |
| **Global indexes / unique constraints** expensive or restricted. |

**Would hurt:** expectation “wire-compatible PG and linear scale with zero app change.”  
**Mitigation:** honest subset; force locality; global unique as special case.

### 1.7 etcd / DCS path

| Risk | Why |
|------|-----|
| **Raft single-group** (etcd-like) still **write-throughput limited** — fine for Patroni, bad if used as app DB. |
| Watch fanout / lease churn at k8s scale is hard (etcd specialization). |

**Would hurt:** replacing etcd for **full Kubernetes** control plane early.  
**Mitigation:** target Patroni/small DCS first; k8s only with serious compatibility + scale work.

### 1.8 “N writers” ceiling

| Risk | Why |
|------|-----|
| **Hot region / hot key** | Still one leader; N writers doesn’t help one hot row. |
| **Leader imbalance** | Bad placement → few nodes take all writes. |

**Would hurt:** celebrity keys, un-split hot ranges.  
**Mitigation:** split, key design, load-aware PD.

---

## 2. Correctness & semantics

### 2.1 OCC (when multi-writer)

| Risk | Why |
|------|-----|
| High abort under contention | TiDB moved to pessimistic default. |
| Clients must **retry** correctly (idempotency). |

### 2.2 Snapshot retention / long TX

| Risk | Why |
|------|-----|
| Long read TX blocks version GC → space growth. |
| Eventually need “snapshot too old” (FDB-style pain locally milder but real). |

### 2.3 Prefix layout discipline

| Risk | Why |
|------|-----|
| No physical keyspaces → bad prefixes = bad locality, hard “drop tenant.” |
| Layers can corrupt each other’s key space without namespaces / tenancy layer. |

### 2.4 Apply vs interactive TX

| Risk | Why |
|------|-----|
| Raft apply must not OCC-abort mid-apply. |
| Two paths (TX vs apply_batch) can **diverge** in durability bugs if not unified. |

### 2.5 Distributed SQL isolation surprises

| Risk | Why |
|------|-----|
| Apps written for single-node PG isolation break under distributed retries. |
| Clock / ordering (HLC vs TSO) edge cases. |

---

## 3. Security

### 3.1 Kernel is not a security boundary (FDB lesson)

| Risk | Why |
|------|-----|
| PedraDB is a **library** — any code in the process can read the files if OS allows. |
| No multi-tenant ACL in core by design. |

**Would hurt:** untrusted plugins in the same process; “DB user permissions” expected in kernel.  
**Mitigation:** process isolation, encrypt at rest in layer/OS, auth at **API layer** (SQL/DCS), not in PedraDB.

### 3.2 Encryption / compliance

| Risk | Why |
|------|-----|
| At-rest encryption, key rotation, audit — usually **not** in v1 kernel. |
| Wire TLS lives in gRPC/SQL layers, not PedraDB. |

**Would hurt:** regulated multi-tenant SaaS before layers exist.  
**Mitigation:** plan encryption as layer or process-level (disk, fs).

### 3.3 Multi-tenant “safe sharing”

| Risk | Why |
|------|-----|
| Prefix tenancy is soft isolation — bug in layer = cross-tenant read. |
| True isolation needs separate processes/Dbs or strong tenancy layer. |

### 3.4 Supply chain / unsafe

| Risk | Why |
|------|-----|
| `forbid(unsafe)` helps but deps and OS still matter. |
| If we ever add unsafe for perf, audit burden rises (sled path). |

---

## 4. Operations & reliability

### 4.1 Young engine risk

| Risk | Why |
|------|-----|
| Corruption bugs, recovery holes — **all new stores** hit this. |
| Format migrations pain (sled warning). |

**Would hurt:** early production without backup story.  
**Mitigation:** crash tests, sim (later), backup/export early, conservative format versioning.

### 4.2 Backup / PITR

| Risk | Why |
|------|-----|
| Distributed backup across M regions is a **product**, not free with PedraDB. |
| Local snapshot alone ≠ cluster-consistent backup without coordination. |

### 4.3 Observability

| Risk | Why |
|------|-----|
| Compaction, WAL, TX abort metrics needed before “production substrate.” |
| Thin kernel can still be opaque without hooks. |

### 4.4 Single-process multi-thread only

| Risk | Why |
|------|-----|
| Can’t share one data dir across processes (by design). |
| Some deploy patterns (many workers each opening DB) need **one writer process** or multiple directories. |

---

## 5. Product / strategy fit

### 5.1 Compatibility tax

| Target | Where plan may “not work well enough” |
|--------|--------------------------------------|
| **Full Postgres** | Extensions, PL/pgSQL, catalog quirks — years; apps still break |
| **Full etcd for k8s** | Watch/lease scale + exact semantics |
| **True Scylla product** | **Fundamental mismatch** (AP multi-master) — plan will never become Scylla without a second product line |
| **Scylla *need* (control plane)** | **Not a limitation of physics** — multi-Raft + watch replaces that need; see `scylla-need-replacement.md` |
| **SQLite ABI drop-in** | Almost never worth it |

### 5.2 Time-to-value

| Risk | Why |
|------|-----|
| Grail story is **long**; without a near product (embed or Patroni DCS), engine looks academic. |
| fjall/redb ship today. |

### 5.3 Team scope

| Risk | Why |
|------|-----|
| Kernel + DCS + multi-Raft + SQL is multiple companies’ worth of work. |
| Scope creep back into PedraDB core kills the doctrine. |

### 5.4 Hot-key and global secondary indexes

| Risk | Why |
|------|-----|
| Horizontal SQL users want global unique indexes → cross-region tax or restrictions. |
| Celebrity rows don’t scale with N writers. |

---

## 6. Security + performance together (common late surprises)

| Scenario | Failure mode |
|----------|----------------|
| Multi-tenant SQL SaaS on shared PedraDB process | Isolation + noisy neighbor + no row-level security in kernel |
| Every small TX fsync on network-attached disk | Latency SLO miss; “DB is slow” |
| ORM chatty TX across regions | Aborts + multi-ms commits |
| Encrypt everything in userspace without design | Double buffering, CPU bound |
| Large blobs in LSM without value log | Compaction IO storm |

---

## 7. Early warning signals (we’re on the wrong path)

| Signal | Interpretation |
|--------|----------------|
| P0 TX still not demoable after long effort | Engine bet failing; consider fjall facade |
| Must add CF zoo, merge ops, filters to match adopters | Becoming worse fjall |
| Primary customer needs AP multi-master | Wrong primitive; use Scylla line |
| Primary customer needs &lt;100µs durable commit always | Wrong default physics; specialized hardware or accept async |
| SQL wire project blocks kernel quality | Wrong sequencing; freeze syntax, finish primitive |
| Format break every month | Sled mode — stop features, stabilize |

---

## 8. What the plan **is** good for (so limits are in context)

Works well when goals are:

- Embed or single-node ACID KV/SQL-subset  
- CP distributed systems (elections, TX KV, horizontal SQL with locality)  
- Layers that need **correct multi-key updates**  
- **N writers** as **many region leaders**, not same-key multi-master  

Works poorly when goals are:

- Scylla-class AP + same-key multi-writer  
- Redis-class pure memory latency  
- Full PG/etcd/k8s compatibility on day one  
- Untrusted multi-tenant in one process without a security layer  

---

## 9. Mitigations checklist (summary)

| Area | Do |
|------|-----|
| Perf durable | Group commit; optional async; SSD guidance |
| Perf LSM | Benches; value log threshold; solid compaction before fancy |
| Concurrency | OCC later; shard above for scale |
| Dist TX | Locality; short TX; measure abort rate |
| Security | AuthZ at API layer; process isolation; encrypt at rest plan |
| Ops | Crash tests; backup story; metrics; format versions |
| Strategy | One upper product after P0; Scylla out of kernel goals |
| Honesty | Plug types (wire vs plugin vs subset); no “replaces everything” |

---

## 10. Bottom line

The plan can fail later on:

1. **Latency/throughput** if every commit fsyncs without group commit, or LSM amp/compaction ignored.  
2. **Dist SQL / chatty TX** latency and aborts.  
3. **Hot keys / global indexes** not fixed by “N writers.”  
4. **Security** if people treat the library as multi-tenant ACL boundary.  
5. **Compatibility ambition** (full PG/etcd/k8s/Scylla).  
6. **Time** — grail is multi-year; kernel must justify itself early.  
7. **Scylla-shaped products** — structural mismatch, not a tuning issue.

None of that kills a **focused** PedraDB (tiny ACID embed + substrate for CP products).  
It kills a fantasy of **one switch that replaces Postgres + etcd + Scylla at once with zero tradeoffs.**
