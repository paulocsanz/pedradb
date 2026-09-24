# Competitive landscape (detailed): engines in PedraDB’s niche

> Who else builds **local embedded storage** (RocksDB’s job) — especially in
> Rust — and which of them are actually **multi-node products**?
>
> **Short answer on multi-node:** almost all of the “Rust engine peers” are
> **local libraries only**. Multi-node shows up in **products that *use* those
> engines** (SurrealDB, TiKV, BonsaiDb), not in the engines themselves — the
> same split we want for PedraDB.
>
> Sources: project READMEs (fetched 2026-08-11). Snapshot, not a living
> scoreboard of stars/versions.

---

## 1. What PedraDB is (so comparison is fair)

| PedraDB is | PedraDB is not |
|------------|----------------|
| Embedded **library** | Network server |
| **Local** ordered KV (+ planned local multi-key ACID) | Multi-Raft / cluster product |
| Substrate role (**RocksDB / Redwood**) | TiKV / FDB product role |
| Pure Rust, local disk LSM (target) | SQL product, Redis, object-store-first DB |

Comparisons below mark each project as:

- **LOCAL** — library / single process (may allow multi-thread)
- **MULTI-NODE product** — ships cluster/replication as the product
- **HYBRID** — library *and* optional server/cluster modes
- **CLOUD-SHAPED local** — “embedded” API but durability is remote object store (not a Raft cluster of peers, but not single-disk either)

---

## 2. Multi-node cheat sheet (the question you asked)

| Project | Multi-node? | What that means |
|---------|-------------|-----------------|
| **PedraDB** | **No** | Local library only |
| **fjall** | **No** | Explicitly *not* a standalone server |
| **SurrealKV** | **No** | Embedded engine; cluster is **SurrealDB** product |
| **redb** | **No** | Embedded ACID KV |
| **heed / LMDB** | **No** | Embedded; multi-*process* readers via mmap, not a cluster |
| **AgateDB** | **No** | Experimental local engine for TiKV; **TiKV** is multi-node |
| **wickdb** | **No** | Pure Rust LSM embed |
| **parity-db** | **No** | Local blockchain state store; node software is separate |
| **nebari** | **No** | Local TX layer; **BonsaiDb** can network |
| **raft-engine** | **No** | Local log store *for* Multi-Raft (TiKV); not the cluster |
| **rust-rocksdb** | **No** | FFI to RocksDB library |
| **RocksDB / Pebble / Redwood** | **No** | Local engines only |
| **SlateDB** | **Not peer-cluster** | Embedded API; data on **object storage** (shared durable medium) — replication via object store, not Raft nodes of SlateDB |
| **Tonbo** | **Not peer-cluster** | Embedded/serverless; data on **S3**; “no server to manage” |
| **sled** | **No** | Embedded (stale) |
| **SurrealDB** | **Yes (product)** | Can run distributed cluster; **uses** SurrealKV/RocksDB-class engines underneath |
| **TiKV** | **Yes** | Multi-Raft distributed KV; **uses** RocksDB (+ raft-engine) |
| **FoundationDB** | **Yes** | Distributed TX KV; **uses** Redwood locally |
| **BonsaiDb** | **Optional network** | Local and/or QUIC/WebSocket server; not the same as TiKV multi-Raft sharding |
| **CockroachDB / TiDB** | **Yes** | Distributed SQL products |

**Rule of thumb:**  
If it is named like an **engine** (fjall, redb, SurrealKV, RocksDB) → **local**.  
If it is named like a **database product** (TiKV, FDB, SurrealDB, TiDB) → **may be multi-node**, and it **embeds** a local engine.

PedraDB is intentionally in the **engine** column.

---

## 3. Detailed profiles — closest Rust peers

### 3.1 fjall

| Field | Detail |
|-------|--------|
| **URL** | https://github.com/fjall-rs/fjall |
| **Role** | Embeddable LSM key-value engine |
| **Multi-node?** | **No.** README: *“It is not: A standalone database server”* |
| **Network server?** | No |
| **Language** | Pure Rust, safe |
| **Structure** | LSM-tree (RocksDB-like) |
| **API** | Thread-safe BTreeMap-like; multiple keyspaces (column-family analogues) |
| **Transactions** | **Optional**: `OptimisticTxDatabase` (OCC multi-writer serializable) or `SingleWriterTxDatabase` |
| **KV separation** | Optional (large blobs) |
| **Compression** | Built-in (LZ4 default) |
| **Durability** | App chooses `persist` mode; default flush to OS buffers (RocksDB-like), not always fsync |
| **Maturity** | Active, one of the most visible Rust LSM embeds |
| **Used as substrate by a big distributed DB?** | Not as TiKV uses RocksDB; general application embed |

**vs PedraDB**

| Same | Different (PedraDB target) |
|------|----------------------------|
| Local LSM library | TX + research compaction/filters as **defaults**, not only optional |
| Pure Rust | Explicit “outer multi-node DB will embed us” contract |
| Optional TX / KV-sep | Simulation-first culture; Monkey + Lazy Leveling design center |

**Closest competitor in the ecosystem.**

---

### 3.2 SurrealKV

| Field | Detail |
|-------|--------|
| **URL** | https://github.com/surrealdb/surrealkv |
| **Role** | Versioned embedded LSM for **SurrealDB** (reduce RocksDB dependency) |
| **Multi-node?** | **No** (engine). Multi-node is **SurrealDB** |
| **Network server?** | No (library) |
| **Structure** | LSM + value log (Wisckey-style) + GC |
| **Transactions** | **ACID**, snapshot isolation / MVCC, concurrent R/W |
| **Extras** | Time-travel / historical queries, checkpoint & restore, durability levels |
| **Maturity** | Product-backed by SurrealDB org |
| **Substrate story** | Yes — but **for SurrealDB**, not a neutral pillar |

**vs PedraDB**

Very similar *technical* checklist (LSM + ACID + value log).  
PedraDB aims to be **vendor-neutral substrate** + broader research LSM defaults; SurrealKV optimizes for Surreal access patterns and product roadmap.

---

### 3.3 AgateDB (TiKV)

| Field | Detail |
|-------|--------|
| **URL** | https://github.com/tikv/agatedb |
| **Role** | Experimental pure-Rust engine path for **TiKV** (Badger → unistore opts) |
| **Multi-node?** | **No** (engine). **TiKV** is multi-node and still production-defaults to **RocksDB** |
| **Network server?** | No |
| **Structure** | LSM / Badger-like; MVCC (managed mode) |
| **Maturity** | Early / heavy development |
| **Why it exists** | Memory safety + TiKV integration; land unistore optimizations |

**vs PedraDB**

Same *strategic* sentence: “something TiKV-class could use instead of RocksDB.”  
AgateDB is TiKV-shaped and experimental. PedraDB is general-purpose substrate + local TX first.

---

### 3.4 redb

| Field | Detail |
|-------|--------|
| **URL** | https://github.com/cberner/redb |
| **Role** | Simple, portable, high-performance **ACID** embedded KV |
| **Multi-node?** | **No** |
| **Network server?** | No |
| **Structure** | Copy-on-write **B+trees** (LMDB-inspired), not LSM |
| **Transactions** | First-class write/read transactions |
| **Maturity** | **Stable**; format stability promised |
| **Sweet spot** | Embed when you want ACID + pure Rust **today**, not write-amp research |

**vs PedraDB**

Best “ship now” pure-Rust ACID embed for many apps.  
PedraDB only makes sense if **LSM write path + research opts + substrate for heavy write OLTP** matter more than “stable B-tree ACID.”

---

### 3.5 heed (LMDB)

| Field | Detail |
|-------|--------|
| **URL** | https://github.com/meilisearch/heed |
| **Role** | Rust-centric **LMDB** bindings (typed keys/values, ACID) |
| **Multi-node?** | **No** |
| **Network server?** | No |
| **Multi-process?** | LMDB allows **multiple reader processes** via mmap; **single writer** |
| **Structure** | B+tree mmap (C core) |
| **Maturity** | Production (e.g. Meilisearch stack) |
| **vs PedraDB** | Not pure-Rust engine; not LSM; different concurrency model |

---

## 4. Related Rust projects (same neighborhood, different job)

### 4.1 SlateDB — embedded LSM on **object storage**

| Field | Detail |
|-------|--------|
| **Multi-node peer cluster?** | **No** Raft cluster of SlateDB nodes |
| **Shared durability?** | **Yes** — S3/GCS/ABS/… is the shared log of record |
| **API** | Embedded library |
| **Trade-off** | Bottomless + easy “replication” via object store; higher latency/API cost |
| **vs PedraDB** | Different medium (object store vs local disk). Not a RocksDB replacement for TiKV-style local apply |

### 4.2 Tonbo — serverless / edge, Parquet on S3

| Field | Detail |
|-------|--------|
| **Multi-node peer cluster?** | **No** traditional DB cluster |
| **Model** | Stateless compute + S3 data + manifest coordination (“no server to manage”) |
| **vs PedraDB** | Cloud-native/serverless state layer, not local LSM substrate |

### 4.3 sled — cautionary tale (why it “stagnated”)

| Field | Detail |
|-------|--------|
| **URL** | https://github.com/spacejam/sled |
| **Multi-node?** | **No** (embedded) |
| **Crates.io** | Still **`1.0.0-alpha.*`** after years (e.g. alpha.124); never a boring stable 1.0 |
| **Own warnings** | README: if reliability is primary → **use SQLite** (“sled is beta”); space amp vs RocksDB; format **will break** before 1.0; multi-process → use LMDB |
| **Rewrite trap** | README states main is a **large in-progress rewrite** (out of sync with docs); priorities: full storage rewrite via [komora](https://github.com/komora-io) / marble, memory layout rewrite, API changes |
| **Architecture ambition** | Not classic LSM or simple B-tree: lock-free index, custom heap/slab, flush epochs, no traditional WAL — hard to finish and prove |
| **Maintainer model** | Heavily associated with one lead (spacejam); sponsor-funded; high bus factor |
| **Activity** | Repo not archived; occasional commits; **not** “dead GitHub,” but **not production-default** either — long alpha + rewrite = community treats as risky |

**Was sled “safe” (no `unsafe`)?**

| Claim people make | Reality |
|-------------------|---------|
| “Pure Rust” | **Yes** in the sense of **not linking C++ RocksDB** — implemented in Rust |
| “`forbid(unsafe)` / 100% safe Rust” | **No.** Source uses `unsafe` (alloc, heap, metadata, sync primitives, etc. — dozens of `unsafe` tokens on main). Has a serious [SAFETY.md](https://github.com/spacejam/sled/blob/main/SAFETY.md) (STPA hazard analysis) precisely because concurrent + IO + reclaim needs care |
| Memory-safe *by default* vs C++ | Better baseline than C++, but **unsafe blocks still exist**; not the fjall-style “100% safe” marketing |

**Why it feels stagnated (summary):**

1. **Never graduated reliability story** — self-labeled beta/alpha; tells serious users to pick SQLite/RocksDB/LMDB for core constraints.  
2. **Perpetual rewrite** — 0.x production-ish lore vs 1.0 redesign unfinished for a long time → adopters freeze or leave.  
3. **Scope too hard for one-person-class bandwidth** — lock-free + novel on-disk design + TX + compression + …  
4. **Format / migration debt** — breaking disk format before 1.0 scares embedders.  
5. **Space/write economics** — README admits sometimes worse space than RocksDB.  

**Lesson for PedraDB:** justify use with a **small finished kernel** before rewrites and sim theater; don’t ship “champagne of beta” forever; if you use `unsafe`, document it — if you claim `forbid(unsafe)`, mean it (fjall-style).

**Sled DX without sled storage:** map-like / sled-shaped API is a **layer** on PedraDB (`pedra-map`), not a second engine. Full plan (ceilings, option preservation, mapping table, delivery slices):  
[`performance-ceiling-option-preservation-and-sled-layer.md`](performance-ceiling-option-preservation-and-sled-layer.md).

### 4.4 nebari + BonsaiDb

| Field | Detail |
|-------|--------|
| **nebari** | Local transactional storage (alpha); **no** multi-node |
| **BonsaiDb** | Product on nebari: local **and/or** networked (QUIC/WebSockets) |
| **Multi-node?** | BonsaiDb = **optional network server**, not TiKV-style sharded multi-Raft by default |
| **vs PedraDB** | Product-coupled; different storage design |

### 4.5 parity-db

| Field | Detail |
|-------|--------|
| **Multi-node?** | **No** — local store for blockchain clients |
| **Design** | Hash-oriented columns, refcounting, large-batch writes |
| **vs PedraDB** | Specialized keys/workload |

### 4.6 wickdb

| Field | Detail |
|-------|--------|
| **Multi-node?** | **No** (pure Rust LSM embed) |
| **vs PedraDB** | Same broad niche as fjall; smaller ecosystem mindshare |

### 4.7 raft-engine (TiKV)

| Field | Detail |
|-------|--------|
| **Multi-node?** | **No** — local **log** engine for Multi-Raft |
| **Job** | Store Raft logs efficiently (bitcask-like), not general ordered KV for app data |
| **vs PedraDB** | Complementary component in a TiKV-like stack, not a competitor for “RocksDB for user data” |

### 4.8 rust-rocksdb

| Field | Detail |
|-------|--------|
| **Multi-node?** | **No** |
| **What** | Rust FFI to **C++ RocksDB** |
| **vs PedraDB** | Escape hatch / oracle / competitor by dependency, not a rewrite |

---

## 5. Non-Rust substrates and products (context)

| Name | Multi-node? | Role |
|------|-------------|------|
| **RocksDB** | No | Industry-default local LSM library |
| **Pebble** | No | Local LSM for CockroachDB |
| **Titan** | No | WiscKey-style **plugin** on RocksDB (TiKV) |
| **TerarkDB** | No | ByteDance RocksDB fork (C++) |
| **Redwood** | No (alone) | FDB’s local B-tree; not a public embed product |
| **Badger** | No | Go LSM + value log + TX (Dgraph) |
| **TiKV** | **Yes** | Distributed TX KV; embeds RocksDB |
| **FoundationDB** | **Yes** | Distributed TX KV; embeds Redwood |
| **CockroachDB** | **Yes** | Distributed SQL; embeds Pebble |
| **TiDB** | **Yes** | SQL layer on TiKV |
| **SurrealDB** | **Yes (optional cluster)** | Multi-model product; embeds SurrealKV/etc. |
| **etcd** | **Yes (single Raft group)** | Coordination KV, not app-data substrate |

---

## 6. Feature matrix (engines only)

| Project | Local lib | Multi-node product? | LSM | Local multi-key ACID | KV-sep | Pure Rust engine | Active | Notes |
|---------|-----------|---------------------|-----|----------------------|--------|------------------|--------|-------|
| **PedraDB** | Yes | **No** | Yes | Target: yes core | Target: WiscKey | Yes | Early | Neutral substrate |
| fjall | Yes | **No** | Yes | Optional | Optional | Yes | Yes | Closest peer |
| SurrealKV | Yes | **No** | Yes | Yes | Yes | Yes | Yes | Surreal-first |
| AgateDB | Yes | **No** | Yes | MVCC | Badger-like | Yes | Exp. | TiKV-first |
| redb | Yes | **No** | No (B+tree) | Yes | N/A | Yes | Stable | Ship-today ACID |
| heed | Yes | **No** | No (LMDB) | Yes (LMDB) | N/A | Bindings | Stable | C core |
| SlateDB | Yes | Object-store shared | Yes | Limited | Planned | Yes | Yes | Not local-disk RocksDB |
| Tonbo | Yes | S3-centric | Parquet/S3 | MVCC-ish | N/A | Yes | Yes | Serverless |
| sled | Yes | **No** | Custom | Partial | No | Yes | Stale | Cautionary |
| parity-db | Yes | **No** | Custom | Batches | Size tables | Yes | Yes | Chain-specific |
| wickdb | Yes | **No** | Yes | ? | ? | Yes | Smaller | Peer LSM |
| raft-engine | Yes | **No** | Log | Batches | N/A | Yes | Yes | Raft logs only |
| RocksDB | Yes | **No** | Yes | No (core) | BlobDB opt | C++ | Yes | Default substrate |
| Pebble | Yes | **No** | Yes | No (core) | No | Go | Yes | Under CRDB |

---

## 7. Products that *are* multi-node (and what they embed)

```
┌──────────────────┐     embeds      ┌─────────────────┐
│ TiKV (multi-node)│ ───────────────►│ RocksDB (local) │
│                  │                 │ raft-engine     │
└──────────────────┘                 └─────────────────┘

┌──────────────────┐     embeds      ┌─────────────────┐
│ FDB (multi-node) │ ───────────────►│ Redwood (local) │
└──────────────────┘                 └─────────────────┘

┌──────────────────┐     embeds      ┌─────────────────┐
│ SurrealDB        │ ───────────────►│ SurrealKV / …   │
│ (optional cluster)│                └─────────────────┘
└──────────────────┘

┌──────────────────┐     embeds      ┌─────────────────┐
│ Future our DB    │ ───────────────►│ PedraDB (local) │  ← intended split
│ (multi-node)     │                 └─────────────────┘
└──────────────────┘
```

**None of fjall / redb / SurrealKV / AgateDB / heed are multi-node products.**  
They are the **left-hand box empty** — the engine column.

---

## 8. Implications for PedraDB (detailed)

### 8.1 You are not alone

Building “Rust embedded KV” is a **crowded idea**. fjall and SurrealKV are real.

### 8.2 Multi-node is not what differentiates you from them

Almost all peers are **also local-only**.  
Saying “we’re local only” does **not** separate PedraDB from fjall — it **aligns** you with them and with RocksDB.

What *would* wrongly differentiate you: adding multi-node to PedraDB and competing with TiKV/FDB as a product.

### 8.3 Real wedges (must be earned)

1. **Research LSM defaults** — WiscKey + Monkey + Lazy Leveling together  
2. **Local multi-key ACID as core contract** for substrate consumers (not only optional)  
3. **Neutral pillar** — not Surreal-only, not TiKV-only  
4. **Simulation / crash discipline** from day one  
5. **Clear apply/batch API** for a future outer multi-node DB  

### 8.4 What to track monthly

| Peer | Watch for |
|------|-----------|
| fjall | TX defaults, KV-sep maturity, production adopters |
| SurrealKV | Features landing in SurrealDB; generality of API |
| redb | Performance vs LSM for write-heavy benches |
| AgateDB | Whether TiKV ever moves off RocksDB |
| SlateDB | Only if object-store substrate becomes a goal (currently no) |

### 8.5 When *not* to build PedraDB

- If “good enough pure Rust LSM + optional TX” = **fjall**  
- If “stable ACID B-tree” = **redb**  
- If “Surreal-shaped” = **SurrealKV**  
- If “just use C++” = **RocksDB** via rust-rocksdb  

Build PedraDB only if the **substrate + research LSM + local ACID** story is the product.

---

## 9. FAQ

### Do fjall / redb / SurrealKV have multi-node?

**No.** Local libraries. (fjall explicitly is not a server.)

### Does SurrealDB have multi-node?

**Yes (optional distributed mode).** That is the **product**, not SurrealKV.

### Does AgateDB make TiKV pure-Rust end-to-end?

**Not in production today.** TiKV still uses RocksDB; AgateDB is experimental.

### Is SlateDB multi-node?

**Not like TiKV.** Many clients can share object storage; there is no SlateDB Raft membership product in the RocksDB sense. Durability/replication is the object store’s job.

### Should PedraDB add multi-node because FDB has it?

**No.** FDB is the *product* class. PedraDB is the *engine* class. Multi-node belongs to a **future separate DB** that embeds PedraDB.

---

## 10. Sources

| Project | Source |
|---------|--------|
| fjall | github.com/fjall-rs/fjall README (“not a standalone database server”) |
| SurrealKV | github.com/surrealdb/surrealkv README |
| SurrealDB | github.com/surrealdb/surrealdb README (embedded or distributed cluster) |
| redb | github.com/cberner/redb README |
| AgateDB | github.com/tikv/agatedb README |
| SlateDB | github.com/slatedb/slatedb README |
| Tonbo | github.com/tonbo-io/tonbo README |
| sled | github.com/spacejam/sled README |
| parity-db | github.com/paritytech/parity-db README |
| nebari / BonsaiDb | github.com/khonsulabs/{nebari,bonsaidb} README |
| heed | github.com/meilisearch/heed README |
| raft-engine | github.com/tikv/raft-engine README |
| Titan | github.com/tikv/titan README |
| TerarkDB | github.com/bytedance/terarkdb README |

Related PedraDB docs: `architecture-refined.md`, `engine-landscape-and-ideal-path.md`,
`distribution-design.md` (outer DB research only).
