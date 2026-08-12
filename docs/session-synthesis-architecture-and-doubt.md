# Session synthesis: architecture, competitors, durability, and “is PedraDB wrong?”

**Status:** working journal (not normative alone — see RFC-0001 for product spec)  
**Updated:** 2026-08-11  
**Purpose:** Persist everything discussed so decisions and doubts don’t live only in chat.  
**Later synthesis (learnings + P0 conflict check):** [`conversation-learnings-and-short-term-alignment.md`](conversation-learnings-and-short-term-alignment.md)

---

## 1. Where we landed (product picture)

### PedraDB is

| | |
|--|--|
| **What** | Pure-Rust **embedded library** |
| **API target** | `open → begin → get/put/delete/range → commit` |
| **Superpower** | **Multi-key ACID** on **ordered** keys, **one process** |
| **Disk** | LSM-family store under the hood |
| **Role** | **Substrate** — same job as **RocksDB** under TiKV or **Redwood** under FDB, ideally **with** local TX so outer systems don’t bolt TX onto a mute engine |

### PedraDB is not

| | |
|--|--|
| Multi-node / Raft / PD / gRPC server | That’s a **future separate product** that would *embed* PedraDB per node |
| Multi-process open of the same directory | Wrong model for this class (unlike LMDB readers) |
| SQL / indexes / documents in core | Layers |
| “FDB reimplemented as a cluster in this repo” | FDB is a **product class**; PedraDB is an **engine class** |
| Feature race with fjall | If we grow surface to match fjall without a clearer kernel story → **use fjall** |

### Stack picture (agreed correction)

```
┌────────────────────────────────────────────┐
│  App  OR  future multi-node DB (not this)  │
│  e.g. “our TiKV/FDB-class product later”   │
└─────────────────┬──────────────────────────┘
                  │ links library
┌─────────────────▼──────────────────────────┐
│  PedraDB                                   │
│  ordered KV + multi-key ACID               │
│  1 process · 1 directory · no network      │
└─────────────────┬──────────────────────────┘
                  ▼
                disk
```

**Important correction mid-conversation:** PedraDB itself does **not** “become multi-node later.” Multi-node is **another codebase/product** that uses PedraDB like TiKV uses RocksDB.

---

## 2. How we got here (conversation arc)

1. Started from RocksDB reimplementation / FDB limits / distributed systems research.  
2. Clarified: **FDB ≈ TiKV** (distributed TX KV), **not ≈ RocksDB** (local engine).  
3. FDB local store = **Redwood (B-tree)**; TiKV local = **RocksDB (LSM)**; PedraDB local = **own LSM**.  
4. Distribution research (multi-Raft, Percolator, Parallel Commits, etcd, Scylla, TiDB) kept as **research for an outer product**, not PedraDB scope.  
5. Competitive landscape: Rust embed space is **crowded** (fjall, SurrealKV, redb, heed, AgateDB, SlateDB, Tonbo, sled…).  
6. Positioning: **tiny surface, speed, build-on potential**; **justify use first**; simulation/papers **later**.  
7. RFC-0001 high-level spec + open decisions O1–O10 + deep dive.  
8. Durability debate: fsync on commit vs RocksDB/fjall defaults; TiKV protected by **Raft quorum**, not by RocksDB sync=true on every put.  
9. **Current emotional/strategic state:** maybe PedraDB is the wrong approach; document everything and sit with the doubt.  
10. **Grail ladder + etcd/Patroni DCS + plug map + switch bar** — layers doctrine; Scylla product vs Scylla *need*.  
11. **P0 implementation start:** RFC-0002; InternalKey + MemTable shipped; WAL was already P0.1.  
12. **SQL world:** TiDB vs Postgres/MySQL; Aurora/Neon log-is-DB; Vitess/Citus; Spanner TrueTime; Rung 1.5; WAL export → Must.  
13. **WAL-as-storage primitive:** sequence + snapshot get already in code; pageserver *role* is outer product; PedraDB materializes.  
14. **Object storage possibility:** kernel still no; Rung 1.5 export open (SlateDB/WarpStream/turbopuffer/Tigris researched).  
15. **Alignment:** research does **not** conflict with short-term P0 — see `conversation-learnings-and-short-term-alignment.md`.

---

## 3. Comparisons that matter

### 3.1 FDB vs TiKV vs RocksDB vs PedraDB

| | RocksDB | FDB | TiKV | PedraDB (target) |
|--|---------|-----|------|------------------|
| Deploy | Library | Cluster product | Cluster product | Library |
| Local engine | Itself (LSM) | Redwood (B-tree) | RocksDB | Own LSM |
| Multi-key ACID in engine | No | Yes (system-wide) | Bolt-on (Percolator) | Yes (local) |
| Multi-node | No | Yes | Yes | No |
| Closest role to PedraDB | **Yes (role)** | Product above | Product above | — |

### 3.2 LSM vs B-tree (short)

- **B-tree:** update in place / COW; great reads/scans; used by redb, LMDB, Redwood.  
- **LSM:** append + compact; great write ingest; used by RocksDB, fjall, PedraDB target.  
- PedraDB chose LSM to be in the **RocksDB substrate** family, not LMDB/redb family.

### 3.3 fjall (closest peer) — nuances

| | fjall | PedraDB target |
|--|-------|----------------|
| Maturity | Shipped 3.x, community | Early (WAL only) |
| API | Database + keyspaces + optional Tx modes | TX-first tiny API only |
| TX | Optional (OCC or single-writer) | Core identity |
| Safe Rust | 100% safe claim | forbid(unsafe) |
| Server / multi-node | No | No |
| Durability default | OS buffers; `persist(Sync*)` explicit | RFC leans fsync on commit for P0 |
| Keyspaces | Physical LSM per keyspace | Prefixes only (v1) |

**If PedraDB becomes “fjall with more options,” PedraDB is wrong — use fjall.**

### 3.4 sled (cautionary)

- Long **alpha**, self-says use SQLite for reliability; format breaks before 1.0.  
- Perpetual **rewrite** (komora/marble); high ambition (not simple LSM).  
- **Pure Rust but not forbid(unsafe)** — uses `unsafe` + SAFETY.md.  
- Lesson: don’t live in rewrite; ship a small useful kernel; don’t overclaim.

### 3.5 Multi-node among “competitors”

Almost all **engines** (fjall, redb, SurrealKV, AgateDB, heed, RocksDB, Pebble): **no multi-node**.  
**Products** (TiKV, FDB, SurrealDB cluster, TiDB): multi-node and **embed** an engine.

---

## 4. Durability: machine kill vs process kill

| Event | Typical `sync=false` WAL (RocksDB/LevelDB/fjall default) |
|-------|----------------------------------------------------------|
| Kill process only | Usually **no** data loss (data already in OS page cache) |
| Kill machine / power loss | May lose **last un-fsynced** writes |

### How peers protect against **machine** kill

| System | Protection |
|--------|------------|
| **LevelDB/RocksDB** | Optional `sync=true`; batch + periodic sync; accept loss; or app-level reconstruction |
| **fjall** | `persist(Buffer \| SyncData \| SyncAll)`; Drop tries journal sync — **not** full power-loss safety unless Sync* used |
| **TiKV** | **Raft**: client commit after **majority** has durable log; one machine dead ≠ lost committed write (if quorum intact). Not “RocksDB fsync every put.” |
| **etcd** | Single Raft group + fsync culture → correct but **write-throughput limited** / disk heavy by design (metadata store) |
| **Postgres/SQLite** | fsync-on by culture for “real DB” |

### PedraDB implication

- **No Raft shield** (single process).  
- Machine-kill safety ⇒ **local fsync/fdatasync** (or honest “may lose tail”).  
- fsync every commit without **group commit** can be **much** slower (LevelDB: async can be ≫1000× sync in extreme cases).  
- With **group commit**, under concurrent commits, cost amortizes — **not automatically “as heavy as etcd”** (etcd multiplies fsync × replicas × single group).  
- Tradeoff still open (O1 in RFC).

---

## 5. RFC-0001 open decisions (O1–O10) — compressed

Full deep dive: `docs/rfc/0001-open-decisions-deep-dive.md`.

| ID | Topic | Peer practice | Lean for P0 |
|----|--------|---------------|-------------|
| O1 | Commit durability | Engines default async; SQL DBs default sync | **Sync WAL on commit** + later relaxed/group |
| O2 | Write concurrency | redb single-writer; fjall both; TiDB moved to pessimistic | **Single-writer TX P0**, OCC later |
| O3 | Keyspaces | RocksDB CF / fjall KS vs FDB prefixes | **Prefixes only** |
| O4 | Interactive TX vs batch | Both eventually | **Interactive TX** first; apply_batch later |
| O5 | Snapshot timing | Usually at begin | **At begin** |
| O6 | Compaction strategy | Simple first in production culture | **Simple correct P1**; research P2 |
| O7 | Value log | Optional; GC painful | **Inline P0–P1** |
| O8 | Size limits | FDB tiny; RocksDB huge | Soft caps TBD |
| O9 | Bindings | Rust engines Rust-first | **Rust only** |
| O10 | Name | “DB” vs engine | Keep name; message kernel |

**Locked already:** local only, no multi-process, tiny TX API, LSM, justify-use before sim, forbid unsafe, multi-node = other product.

---

## 6. Delivery phasing (justify use first)

| Phase | Goal | Not yet |
|-------|------|---------|
| **A / P0** | Multi-key TX + crash recovery demo | Full SST suite, sim, papers as pitch |
| **B / P1** | SST, range merge, compaction correctness, benches | Cluster |
| **C–D / P2** | Speed opts, sim, oracle, substrate hooks | — |
| **E** | Outer multi-node product embeds PedraDB | Inside this repo as PedraDB features |

**P0 success:**  
`begin → put(row)+put(index) → commit` → kill → reopen → still consistent; tiny docs.

---

## 7. “Maybe PedraDB is the wrong approach” — honest case analysis

### 7.1 Arguments **against** building PedraDB

| Argument | Force |
|----------|--------|
| **fjall exists** | Same job class (safe Rust LSM embed), more mature, TX available, community |
| **redb exists** | Stable pure-Rust ACID embed if B-tree is enough |
| **SurrealKV exists** | LSM+ACID+value log, product-backed |
| **RocksDB exists** | Industry default substrate; rust-rocksdb if you accept C++ |
| **AgateDB / TiKV path** | “Replace RocksDB under distributed KV” is already someone’s experiment |
| **Sled trauma** | Ecosystem wary of ambitious Rust embed DBs that never leave alpha |
| **fsync vs speed** | Safe ACID local kernel may **lose** microbench vs fjall defaults; hard marketing |
| **We don’t have a consumer yet** | Substrate without an outer DB or app is a bet on future use |
| **Scope creep risk** | Chat already drifted to multi-node, sim, papers — classic way to never ship |

### 7.2 Arguments **for** still building PedraDB

| Argument | Force |
|----------|--------|
| **TX-first tiny surface** | fjall is a flexible kit; dual Database/TxDatabase mental model; we want one kernel face |
| **Neutral substrate** | Not Surreal-only, not TiKV-only |
| **Local ACID under future multi-node** | TiKV paid years bolting Percolator on RocksDB; we want local TX before outer product |
| **Research defaults under the hood** | Optional later; only if measured — not the pitch |
| **Learning / ownership** | Full control of format and recovery for a company stack |
| **Clear non-goals** | If we hold them, we avoid sled/fjall-clone failure modes |

### 7.3 Alternative strategies (if PedraDB is “wrong”)

| Alternative | When it wins |
|-------------|--------------|
| **A. Use fjall** | Need embed LSM+optional TX **now**; don’t need own engine |
| **B. Use redb** | ACID embed, B-tree OK, stability first |
| **C. Use RocksDB (rust-rocksdb)** | Need production LSM substrate tomorrow; accept C++ |
| **D. Thin wrapper / policy layer on fjall** | Want “TX-mandatory API” without writing LSM — facade that only exposes TX API over fjall |
| **E. Skip engine; build outer multi-node on RocksDB/fjall** | Real goal is distributed product, not engine |
| **F. Continue PedraDB but freeze scope hard** | Still believe in kernel TX + own LSM; ship P0 only |

**Option D** is a serious “doubt resolver”: validate the **product API and layers** without reimplementing LSM. If the facade never needs to leave fjall, PedraDB engine was unnecessary. If fjall blocks (format, hooks, perf, license, philosophy), then own engine is justified.

### 7.4 Decision criteria (suggested)

Continue PedraDB engine **only if** at least two hold:

1. We will **not** accept fjall/redb as long-term substrate for **our** stack (control, hooks, or philosophy).  
2. We will ship **P0 TX demo in a short horizon** (weeks, not “after sim and papers”).  
3. We have a **concrete first consumer** (app or planned outer DB) that needs multi-key ACID embed.  
4. We accept that **near-term** we lose to fjall on maturity and maybe raw default-async benches.

If (1)–(3) fail → **use fjall (or D)** and keep research docs as future insurance.

---

## 8. Document map (what we wrote this session / project)

| Path | Content |
|------|---------|
| `docs/rfc/0001-pedradb-high-level-spec.md` | **Normative** high-level spec, P0/P1/P2, locked vs open |
| `docs/rfc/0001-open-decisions-deep-dive.md` | O1–O10 peers, nuances, regrets |
| `docs/positioning.md` | Justify use first; power/surface |
| `docs/architecture-refined.md` | Local only; outer multi-node separate |
| `docs/architecture.md` | Mission + roadmap |
| `docs/compare-fjall.md` | PedraDB vs fjall + nuances |
| `docs/competitive-landscape-rust.md` | Rust peers, multi-node table, sled |
| `docs/fdb-limitations-analysis.md` | FDB limits + more API limits |
| `docs/distribution-design.md` | Outer multi-Raft research (not PedraDB features) |
| `docs/distribution-deep-research.md` | Percolator, Parallel Commits, PD, etc. |
| `docs/etcd-comparison.md` | etcd vs everyone |
| `docs/scylladb-architecture.md` | Scylla (AP multi-master) |
| `docs/scylla-need-replacement.md` | Replace Scylla *need* (mono networking/orch), not product |
| `docs/tidb-vs-postgres-mysql.md` | Monolith SQL vs TiDB distributed |
| `docs/sql-lessons-for-the-grail.md` | Aurora/Neon/Vitess/Citus/Spanner → ladder |
| `docs/object-storage-as-substrate-possibility.md` | S3 as Rung 1.5 medium (open), not kernel |
| `docs/conversation-learnings-and-short-term-alignment.md` | All learnings + P0 conflict matrix |
| `docs/rfc/0002-internal-key-memtable.md` | P0.2 done |
| `docs/tidb-architecture.md` | TiDB as SQL layer on TiKV |
| `docs/foundationdb-layers-and-products.md` | What runs on FDB |
| `docs/engine-landscape-and-ideal-path.md` | Broader engine survey |
| `docs/session-synthesis-architecture-and-doubt.md` | **This file** |

Code today: WAL (P0.1 done). Everything else is design.

---

## 9. Open questions for the human (decision log)

Record answers when decided:

| # | Question | Answer (fill in) |
|---|----------|------------------|
| Q1 | Is the real goal an **engine** or a **multi-node DB**? | |
| Q2 | Will we **refuse** fjall as substrate for the long term? Why? | |
| Q3 | First **concrete consumer** of PedraDB (app / product)? | |
| Q4 | P0 durability: **sync commit** or **async default**? | |
| Q5 | P0 concurrency: **single-writer** or OCC? | |
| Q6 | Proceed with own LSM, **facade over fjall**, or **stop**? | |
| Q7 | Time box for P0 TX demo? | |

---

## 10. One-page summary

- PedraDB = **local ACID ordered KV library**, not a cluster.  
- Multi-node = **other product** embedding PedraDB (like TiKV→RocksDB).  
- Closest peer = **fjall** (more mature); wedge = **TX-first tiny kernel**, not “also an LSM.”  
- TiKV survives machine kill via **Raft quorum**, not RocksDB sync-every-put.  
- PedraDB has **no quorum**; power-loss safety needs **local fsync** (or honesty).  
- fsync default can be slow; **group commit** avoids etcd-like pathology.  
- Simulation, Monkey, Lazy Leveling = **later**, after justify-use.  
- **Doubt is valid:** without a consumer and a fast P0, **fjall (or a thin TX facade)** may be the rational path.  
- Continuing PedraDB only makes sense with **frozen scope + short P0 + clear “why not fjall.”**

---

## 11. Recommended next step (process, not code)

1. Fill §9 Q1–Q7 in this file or a short reply.  
2. Either:  
   - **A)** Amend RFC-0001 → `approved` + implement P0.2–P0.5, or  
   - **B)** Spike “PedraDB API” as facade over fjall for 1 week, or  
   - **C)** Pause engine; adopt fjall; keep docs as research.  

Do not start simulation framework or multi-node design until Q1–Q3 are clear.
