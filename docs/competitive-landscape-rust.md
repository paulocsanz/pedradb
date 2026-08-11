# Competitive landscape: who else is doing what PedraDB is doing?

> Research snapshot (2026-08-11). PedraDB’s niche: **embedded local ordered KV
> library in Rust**, durable on disk, intended as a **substrate** (RocksDB role)
> that can also offer **local multi-key ACID**, with a research-backed LSM
> (WiscKey + Monkey + Lazy Leveling) and FDB-style layer philosophy.
>
> Others *have* built pieces of this. Almost nobody ships the full combination.

Primary sources: project READMEs / GitHub (fjall, redb, SurrealKV, SlateDB,
Tonbo, sled, AgateDB, parity-db, nebari, wickdb, heed, raft-engine, rust-rocksdb).

---

## TL;DR

| Question | Answer |
|----------|--------|
| Já existe engine LSM embutida em Rust? | **Sim** — fjall, SurrealKV, wickdb, AgateDB (exp.), outros |
| Já existe KV embutido com ACID em Rust? | **Sim** — redb, heed/LMDB, fjall (TX opcional), SurrealKV |
| Alguém é “o RocksDB do TiKV em Rust puro, maduro, default do ecossistema”? | **Não** — TiKV ainda usa RocksDB C++; AgateDB é experimental |
| Alguém combinou WiscKey + Monkey + Lazy Leveling + sim FDB + TX local + papel de pillar? | **Não de forma completa e madura** |
| Devemos desistir? | **Não** — mas o espaço **não está vazio**; o diferencial tem que ser explícito |

---

## Map of the space

```
                    ACID multi-key local TX
                           ▲
              redb · heed  │  SurrealKV · fjall(opt)
              (B-tree)     │  (LSM + TX)
                           │
     pure storage ─────────┼──────────── rich product
     (RocksDB role)        │            (DB product)
                           │
              wickdb       │  sled(?) SurrealDB
              parity-db    │  nebari    (uses SurrealKV)
              AgateDB      │
                           │
                    SlateDB · Tonbo
                    (object store / serverless)
                           │
                           ▼
                    durability / cloud
```

PedraDB aims at the **upper-left → center**: local LSM substrate **with** TX,
usable by a future outer multi-node DB (like RocksDB under TiKV), not a
serverless S3 engine and not a full SQL product.

---

## Closest peers (same job class)

### 1. fjall — closest active Rust LSM peer

| | |
|--|--|
| **What** | Embeddable LSM KV, pure Rust, BTreeMap-like API |
| **TX** | Serializable transactions **optional** |
| **KV-sep** | Optional (large blobs) |
| **Safety** | 100% safe Rust |
| **Status** | Active, growing (~2k+★ on GitHub topic search) |
| **vs PedraDB** | Same niche (local LSM library). PedraDB differentiates on: TX **default**/first-class pillar, Monkey Bloom, Lazy Leveling as design center, explicit “substrate for outer TiKV-class DB”, simulation-first testing culture |

**Verdict:** Real competitor. Must track feature parity and quality bar.

### 2. SurrealKV — LSM + ACID + Wisckey for SurrealDB

| | |
|--|--|
| **What** | Versioned embedded LSM for **SurrealDB** (replace RocksDB dependency) |
| **TX** | ACID, snapshot isolation / MVCC |
| **KV-sep** | Value log (Wisckey-style) + GC |
| **Extras** | Time-travel queries, checkpoint/restore |
| **Status** | Product-backed (SurrealDB org) |
| **vs PedraDB** | Very similar technical goals (LSM + TX + value log). Coupled to SurrealDB evolution; not positioned as universal “pillar for any layer.” No claim of Monkey/Dostoevsky combination |

**Verdict:** Strongest proof that “RocksDB replacement in Rust with TX” is in demand. PedraDB is the **generalist substrate**; SurrealKV is **Surreal-first**.

### 3. AgateDB (TiKV) — experimental RocksDB/Badger path for TiKV

| | |
|--|--|
| **What** | Pure Rust KV; plan: port Badger → unistore opts → TiKV engine |
| **TX/MVCC** | KV with MVCC (managed-mode Badger-like) |
| **Status** | Early / heavy development; not TiKV production default |
| **vs PedraDB** | Same *strategic* idea (“engine TiKV can use instead of RocksDB”). TiKV-specific optimizations; not a general pillar product |

**Verdict:** Validates the “replace RocksDB under a distributed KV” thesis. PedraDB is not TiKV-locked.

### 4. redb — mature pure-Rust ACID, but B+tree not LSM

| | |
|--|--|
| **What** | ACID embedded KV, COW B+trees, LMDB-inspired |
| **Status** | **Stable**, maintained, format stable |
| **vs PedraDB** | Better “production readiness” story today. Different structure (B-tree): weaker write-heavy asymptotics vs LSM research path. Not aimed at WiscKey/Monkey/Dostoevsky |

**Verdict:** Best “just use this” Rust ACID embed today for many apps. PedraDB only wins if LSM + research path + substrate story matters.

### 5. heed (LMDB bindings) — battle-tested C core

| | |
|--|--|
| **What** | Safe Rust API over **LMDB** (mmap B+tree, MVCC, single writer) |
| **Used by** | Meilisearch, etc. |
| **vs PedraDB** | Not pure Rust engine; not LSM; single-writer model. Production proven |

**Verdict:** Different trade-offs (read-heavy, simple). Not the same design space.

---

## Related but different niche

| Project | Niche | Why not the same as PedraDB |
|---------|-------|-----------------------------|
| **SlateDB** | LSM **on object storage** (S3…), bottomless | Cloud object latency/cost model; not local-disk RocksDB replacement |
| **Tonbo** | Embedded for **serverless/edge**; Parquet on S3; Arrow | Stateless compute + S3; not local LSM substrate for TiKV-like DB |
| **sled** | Embedded “B-link” style DB | Effectively **stalled/abandoned** — cautionary tale |
| **nebari** | TX storage for BonsaiDb (Couchstore-inspired) | Alpha; product-coupled; not LSM research path |
| **parity-db** | Blockchain state (hash keys, refcount) | Specialized workload, not general ordered KV pillar |
| **wickdb** | Pure Rust LSM | Smaller mindshare; check maturity before relying |
| **raft-engine** | **Only** Multi-Raft logs (bitcask-like) | Log store, not general KV |
| **rust-rocksdb** | FFI to RocksDB C++ | Not a rewrite; inherits RocksDB design + unsafe boundary |
| **Pebble** (Go) | CRDB’s RocksDB-class engine | Same *role*, different language; no Monkey/Dostoevsky |
| **Badger** (Go) | WiscKey LSM + TX | Closest *paper* cousin; Go; Dgraph stack |

---

## Non-Rust “same problem” (substrate engines)

| Engine | Lang | Role | Notes |
|--------|------|------|-------|
| **RocksDB** | C++ | De-facto substrate | What PedraDB/SurrealKV/AgateDB want to displace |
| **Pebble** | Go | RocksDB-class for CRDB | Clean-room; still classic leveling |
| **WiredTiger** | C | MongoDB | B-tree/LSM hybrid |
| **Redwood** | C++ | FDB local store | B-tree, not a public “embed me” product |
| **LevelDB** | C++ | Ancestor | Too limited for modern substrates |

---

## Feature matrix (local embed focus)

| Project | Lang | Structure | Local multi-key ACID | KV-sep | Pure Rust | Active | Explicit “substrate for distributed DB” |
|---------|------|-----------|----------------------|--------|-----------|--------|----------------------------------------|
| **PedraDB (target)** | Rust | LSM | **Yes (core)** | Yes (WiscKey) | Yes forbid(unsafe) | Early | **Yes** |
| fjall | Rust | LSM | Optional | Optional | Yes | Yes | Partial (general embed) |
| SurrealKV | Rust | LSM | Yes | Yes | Yes | Yes (Surreal) | For SurrealDB |
| AgateDB | Rust | LSM (Badger-like) | MVCC | Badger-style | Yes | Experimental | For TiKV |
| redb | Rust | B+tree COW | Yes | N/A | Yes | Stable | General embed |
| heed/LMDB | Rust+C | B+tree mmap | Yes (LMDB model) | N/A | Bindings | Stable | General embed |
| SlateDB | Rust | LSM→object store | Limited model | Planned | Yes | Yes | Cloud embed |
| Tonbo | Rust | Parquet/S3 | MVCC-ish | N/A | Yes | Yes | Serverless |
| sled | Rust | Custom | Partial | No | Yes | **Stale** | Was “the” hope |
| Pebble | Go | LSM | No (CRDB adds) | No | — | Yes | Under CRDB |
| RocksDB | C++ | LSM | No (limited) | BlobDB opt | — | Yes | Industry default |

---

## Where PedraDB still has a wedge (if we execute)

1. **Combination of research defaults**  
   WiscKey + Monkey Bloom + Lazy Leveling as **default architecture**, not optional experiments. fjall/SurrealKV touch pieces (KV-sep, TX); full trio + cost-model compaction is still rare.

2. **Substrate contract, not a product DB**  
   Explicit API for “outer TiKV-class process will apply batches / use local TX” — AgateDB is TiKV-specific; SurrealKV is Surreal-specific; fjall is general app embed. PedraDB can be **neutral pillar**.

3. **TX in the substrate without being a network DB**  
   RocksDB/Pebble force bolt-on TX (TiKV/CRDB pain). redb has TX but B-tree. SurrealKV/fjall prove demand; PedraDB centers TX + LSM research together.

4. **Deterministic simulation as identity**  
   SlateDB has DST; FDB invented the culture; few local LSMs treat full crash/disk simulation as non-negotiable from day one.

5. **Language + safety**  
   Pure Rust `forbid(unsafe)` vs rust-rocksdb FFI / C++ RocksDB.

### Honest risks

| Risk | Detail |
|------|--------|
| **fjall / SurrealKV ship first** | May cover “good enough” for many users |
| **redb wins simplicity** | If workload isn’t write-amp heavy, B-tree ACID is fine |
| **sled trauma** | Ecosystem skeptical of ambitious Rust embed DBs that stall |
| **TiKV stays on RocksDB** | AgateDB may never replace production RocksDB |
| **Scope creep** | Becoming “mini TiKV” or “mini FDB” kills the substrate focus |

---

## Strategic implications for PedraDB

| Do | Don’t |
|----|--------|
| Stay **library-only, local disk** (RocksDB role) | Ship multi-node “because FDB has it” |
| Make **local ACID** excellent and documented for outer DBs | Compete with Redis features |
| Publish design: Monkey + Lazy Leveling + WiscKey | Silent clone of fjall |
| Oracle-test vs RocksDB | Depend on RocksDB in production |
| Track fjall + SurrealKV releases as peers | Ignore them and reinvent blindly |
| Kill features that don’t serve “substrate + local TX” | Build SQL/server in core |

---

## Recommendation

Your doubt is right: **Rust already has real attempts** (fjall, SurrealKV, redb, AgateDB, SlateDB, Tonbo…).  

PedraDB is **not** inventing “embedded KV in Rust.” It is inventing (or still needs to earn) a **specific corner**:

> Pure-Rust local LSM **substrate** with **first-class multi-key ACID**,  
> research-optimal compaction/filters/KV-sep,  
> simulation-hardened,  
> designed so a **future multi-node DB** embeds it like TiKV embeds RocksDB —  
> without PedraDB itself becoming that multi-node DB.

If that corner is not held tightly, **fjall or SurrealKV** are the rational defaults instead of a greenfield engine.

---

## Sources

| Project | URL |
|---------|-----|
| fjall | github.com/fjall-rs/fjall |
| SurrealKV | github.com/surrealdb/surrealkv |
| redb | github.com/cberner/redb |
| SlateDB | github.com/slatedb/slatedb |
| Tonbo | github.com/tonbo-io/tonbo |
| sled | github.com/spacejam/sled |
| AgateDB | github.com/tikv/agatedb |
| parity-db | github.com/paritytech/parity-db |
| nebari | github.com/khonsulabs/nebari |
| heed | github.com/meilisearch/heed |
| raft-engine | github.com/tikv/raft-engine |
| wickdb | github.com/Fullstop000/wickdb |
| rust-rocksdb | github.com/rust-rocksdb/rust-rocksdb |
| GH search | `lsm embedded language:Rust` (2026-08-11) |
