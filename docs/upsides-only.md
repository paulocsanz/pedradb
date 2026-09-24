# Upsides only: what this architecture unlocks

**Status:** positive case (no “why not Scylla / risks” here)  
**Updated:** 2026-08-11  

PedraDB = powerful **local** primitive (ordered KV + multi-key ACID).  
Products = **API layers** (+ distribution when needed).  
Syntax = thin codecs on deep protocols.

---

## 1. Kernel upsides (PedraDB itself)

| Upside | What you get |
|--------|----------------|
| **Tiny surface** | `open → begin → get/put/delete/range → commit` — easy to learn, hard to misuse |
| **Multi-key ACID in-process** | Row + index, debit + credit, config + version — one commit, no cluster required |
| **Ordered keys + range** | Natural layouts for indexes, queues, tenants, revisions (FDB-style layers) |
| **Embeddable** | Link into the app; no server process for the SQLite-class niche |
| **Pure Rust / forbid(unsafe)** | One language with the rest of a modern stack; audit-friendly kernel |
| **Substrate re-use** | Same engine under embed, DCS, distributed KV, SQL — one recovery story |
| **Durability you can choose** | Strong commit when it matters; async/bulk when loading; group commit for throughput |
| **Correctness path for layers** | Layers don’t reimplement consistency; they compose transactions |

---

## 2. Multi-writer / scale-out upsides (distribution layer on top)

Not multi-master on the **same** key — **better for ACID products:**

| Upside | What you get |
|--------|----------------|
| **N writers** | Many **region leaders** on many nodes accept writes **in parallel** |
| **M regions** | Fine-grained sharding; split hot ranges; rebalance across N nodes |
| **Write scale-out** | Add nodes → place more leaders → more aggregate write throughput |
| **Read scale-out** | Followers / more nodes; optional later: bounded-stale follower reads |
| **Single-region fast path** | TX that fits one region = local PedraDB commit (no 2PC) |
| **Cross-region when needed** | Distributed TX only when keys span regions (pay only when necessary) |
| **Online growth** | Split/merge regions without rewriting the SQL/KV API clients see |
| **Failure isolation** | One region leader fails → election on that range; rest of cluster keeps writing |

```
Node A: leader R1, R4     ── writes for those key ranges
Node B: leader R2, R5     ── parallel writers
Node C: leader R3, R6
= N writers, M regions, one writer per key
```

---

## 3. Plug / ecosystem upsides

| Upside | What you get |
|--------|----------------|
| **Plug where etcd was** | DCS layer (etcd API or Patroni plugin) → same Patroni/HA workflows |
| **Plug where SQLite was** | Embed PedraDB (or thin SQL) for local ACID apps |
| **Plug where MySQL/PG wire was** | Syntax layer → same apps, new horizontal backend |
| **Plug where TiKV was** | Distributed KV API; PedraDB = RocksDB role, with local TX built-in |
| **Replace Scylla *need* (not CQL)** | Scale-out route/WID/orchestrator store + watches → leave Scylla without multi-master product |
| **Many syntaxes, one semantics** | PG wire + MySQL wire + custom RPC on the **same** deep protocol |
| **Deep protocol first** | Ship Patroni/DCS ops before full etcd gRPC; add syntax later without redesign |
| **Incremental compatibility** | Subset wire now; widen dialect over time without changing PedraDB |

---

## 4. Product ladder upsides (one kernel → many DBs)

| Layer product | Upside vs building from scratch |
|---------------|----------------------------------|
| **Embed ACID** | SQLite-class local correctness without a server |
| **DCS / elections** | Patroni-class HA coordination on Raft + PedraDB (bbolt role) |
| **Distributed TX KV** | TiKV/FDB-class platform; local TX simplifies single-region path |
| **Horizontal SQL** | TiDB/CRDB-class: N writers via region leaders, apps still speak SQL wire |
| **Networking / orchestration CP** | Sub-second route & WID maps via multi-Raft + watch (mono Scylla jobs without Scylla) |
| **Shared ops skill** | One storage kernel to operate, backup mental model, test once |

See `scylla-need-replacement.md` for the control-plane scale-out story.

---

## 5. Compared to classic single-primary Postgres

| Upside of horizontal stack on PedraDB |
|---------------------------------------|
| **Multiple write entry points** (region leaders), not one primary bottleneck |
| **Scale storage and writes by adding machines** |
| **Region-level failover** instead of whole-cluster primary only |
| **Stateless SQL frontends** scale compute independently of storage |
| **Same TX story** for data + secondary indexes (distributed when needed) |

---

## 6. Compared to bolting TX onto mute RocksDB (TiKV path)

| Upside |
|--------|
| Multi-key ACID **already in the local primitive** |
| Single-region transactions don’t reinvent MVCC on raw put/get |
| Outer product focuses on Raft + cross-region TX, not “invent storage TX from zero” |
| Embed and distributed products share the **same** local semantics |

---

## 7. Durability / performance upsides (with the right policy)

| Upside |
|--------|
| **Safe default commit** (DataSync) for real ACID apps |
| **Optional async** for bulk load and benches |
| **Group commit** → many TX share one fsync → high throughput without etcd-like multi-node fsync tax |
| **Process-kill safety** even on async path (data in OS); power-loss covered when sync path used |
| Local fsync cost is **one disk**, not RF×Raft unless you add the distribution layer |

---

## 8. Platform / company upsides

| Upside |
|--------|
| **One investment (kernel)** compounds into embed + coord + KV + SQL |
| **Layers ship independently** (DCS team ≠ SQL team; same PedraDB) |
| **Clear ownership** — kernel vs protocol vs syntax |
| **Test pyramid** — prove PedraDB hard; layers become thinner |
| **Narrative** — “FDB-style layers” without forcing every product to be a full FDB cluster |

---

## 9. One-slide upside list

1. Tiny ACID ordered KV embed  
2. Build indexes/models with transactions  
3. Same kernel under every product  
4. **N writers** via **M region leaders** (horizontal write scale)  
5. Fast single-region path + distributed TX only when needed  
6. Plug into Patroni/etcd/SQL wire at the **edge**  
7. Syntax optional on deep protocols  
8. Async when you want speed; sync when you want power-loss safety  
9. Group commit without multi-node tax  
10. Path from SQLite-class → etcd-class → TiKV-class → horizontal Postgres-class  

---

## Bottom line

**Upside of the approach:** a small, strong primitive multiplies into many databases and **scale-out multi-writer** (region leaders), while clients keep plugging into familiar APIs (Patroni/etcd/SQL) via thin layers — without stuffing those protocols into the storage kernel.
