# Plug map: where layers sit so you replace etcd, SQLite, TiKV, TiDB, Postgres, Scylla, …

**Status:** strategy  
**Updated:** 2026-08-11  

**Doctrine:** PedraDB = primitive only.  
**Products** = deep protocol + optional syntax/wire that **plug into existing ecosystems**.

---

## Big picture

```
                    ┌──────────── plug points (what the world already speaks) ────────────┐
                    │                                                                      │
   Patroni ─────────┤ etcd API or DCS plugin                                               │
   App / ORM ───────┤ SQLite API · libsqlite3 · or “just link PedraDB”                     │
   TiDB-like apps ──┤ MySQL wire                                                           │
   CRDB/PG apps ────┤ Postgres wire                                                        │
   k8s / custom ────┤ etcd API · custom KV API                                             │
   CQL apps ────────┤ CQL / Cassandra protocol  (hard — see Scylla)                        │
                    └───────────────────────────────┬──────────────────────────────────────┘
                                                    │ syntax / driver
                    ┌───────────────────────────────▼──────────────────────────────────────┐
                    │  Deep protocol (our semantics)                                       │
                    │  DCS ops · KV TX · SQL ops                                           │
                    ├──────────────────────────────────────────────────────────────────────┤
                    │  Distribution (when needed)                                          │
                    ├──────────────────────────────────────────────────────────────────────┤
                    │  PedraDB (always local storage primitive)                            │
                    └──────────────────────────────────────────────────────────────────────┘
```

**You plug at the boundary the *client already uses*.**  
PedraDB is never that boundary (except pure embed “link the library”).

---

## Plug matrix

| Incumbent | What clients speak today | Where **we** plug | What we run underneath | Drop-in level |
|-----------|--------------------------|-------------------|------------------------|---------------|
| **etcd** | etcd gRPC/HTTP v3 | **Syntax = etcd API** on deep DCS protocol | Raft × N + **PedraDB**/node | High if API-compatible |
| **Patroni → etcd** | Patroni DCS (etcd backend) **or** plugin | **A)** pretend etcd **or** **B)** `AbstractDCS` plugin | Same DCS stack + PedraDB | **B** easier; **A** true replace |
| **SQLite** | C API / SQL file / embed | **Link PedraDB** (no SQL) **or** thin SQL-on-PedraDB with SQLite-like API | PedraDB only (1 process) | Medium: SQL dialect subset; or no SQL, app rewrite to KV TX |
| **Postgres** (1 node) | PG wire + SQL | PG wire syntax → SQL deep protocol → PedraDB | PedraDB on one machine | Hard (compatibility) |
| **Postgres horizontal** | PG wire + SQL | Same wire → distributed SQL → multi-Raft → PedraDB/node | N nodes, M regions | Very hard (CRDB/TiDB scale) |
| **TiKV** | TiKV KV/TX gRPC (or TiDB only) | Deep **distributed KV TX** API (TiKV-like or our own) | multi-Raft + PedraDB/node | Medium: new API or TiKV-compatible surface |
| **TiDB** | **MySQL wire** | MySQL syntax → SQL deep protocol → distributed KV → PedraDB | Same as horizontal SQL | Hard; path proven by TiDB architecture |
| **Scylla / Cassandra** | **CQL** + multi-master semantics | See § Scylla — usually **don’t** claim drop-in | Different consistency model | Low as true Scylla replace |

---

## One card per target

### etcd

| | |
|--|--|
| **Plug** | Port 2379-ish, etcd v3 API (KV, Lease, Watch, TX/CAS) |
| **Who connects** | `etcdctl`, Patroni etcd backend, k8s (if full enough — k8s is strict), custom ops |
| **Stack** | etcd syntax → DCS deep ops → Raft → **PedraDB** |
| **PedraDB role** | bbolt-equivalent state machine storage |
| **Not in PedraDB** | Any etcd RPC |

### Patroni (election / DCS)

| | |
|--|--|
| **Plug option A** | Point Patroni `etcd3.hosts` at **pedra-dcs** etcd API |
| **Plug option B** | Implement Patroni **`AbstractDCS`** backend (`attempt_to_acquire_leader`, `touch_member`, `watch`, …) talking to pedra-dcs deep API |
| **Stack** | Patroni → (etcd wire \| plugin) → Raft → **PedraDB** |
| **Replaces** | etcd (or Consul/ZK) **only as DCS**, not Postgres data |
| **Nuance** | B can ship without full etcd compatibility; A is “plug where etcd was” |

### SQLite

| | |
|--|--|
| **Plug option A** | App **links PedraDB** directly (KV TX API) — not SQL-compatible, API change |
| **Plug option B** | **SQL subset** library API “like SQLite” on PedraDB (same process) |
| **Plug option C** | (Hard) `libsqlite3` ABI shim — usually not worth it |
| **Stack** | App → (SQL layer optional) → **PedraDB** |
| **No** | Raft required |
| **Nuance** | True SQLite drop-in is a huge compatibility project; “alternative for *new* embeds” is realistic first |

### Postgres (single node)

| | |
|--|--|
| **Plug** | Port 5432, Postgres wire + enough SQL |
| **Stack** | PG syntax → SQL deep protocol → **PedraDB** (one node) |
| **Nuance** | Wire + SQL dialect = years; start with subset or non-wire SQL |

### Postgres horizontally scalable (N writers, M regions)

| | |
|--|--|
| **Plug** | Same **PG wire** (apps don’t know about regions) |
| **Stack** | PG syntax → SQL layer → distributed TX → **M regions / multi-Raft** → **PedraDB per node** |
| **N writers** | N **region leaders** (one writer per key range), not multi-master same key |
| **PedraDB** | Local apply/TX only |

### TiKV

| | |
|--|--|
| **Plug** | TiKV-compatible KV/TX API **or** our own gRPC (clients change) |
| **Stack** | KV API → multi-Raft + dist TX → **PedraDB**/node |
| **PedraDB** | Replaces **RocksDB** role inside a TiKV-class process |
| **Nuance** | “Plug as TiKV” needs API parity; “plug as better RocksDB under *our* TiKV” only needs PedraDB |

### TiDB

| | |
|--|--|
| **Plug** | **MySQL wire** (what TiDB apps use) |
| **Stack** | MySQL syntax → SQL → **our** distributed KV (Recipe K) → PedraDB |
| **Nuance** | Same shape as TiDB; we don’t put MySQL inside PedraDB |

### Scylla / Cassandra

| | |
|--|--|
| **Plug people want (product)** | CQL + any-replica writes + tunable CL |
| **Reality** | That **semantic** is multi-master / often eventual — **not** what PedraDB+Raft gives by default |
| **Need people actually have (mono-shaped)** | Scale-out control-plane KV + CAS + sub-second route/WID push — **not** CQL drop-in |
| **Honest options** | (1) Keep Scylla for true AP CQL. (2) **Replace the need** with multi-Raft + PedraDB + watch + gRPC gateway (see `scylla-need-replacement.md`). (3) Optional CQL **syntax** on CP = “Cassandra API, TiKV physics,” not Scylla. |
| **Main line** | Do **not** design kernel for AP multi-master; **do** design L2/L3 so platforms stop *needing* Scylla for networking/orchestration metadata |

---

## “Plug” types (vocabulary)

| Type | Meaning | Example |
|------|---------|---------|
| **Wire drop-in** | Same host:port + protocol | etcd API, PG/MySQL wire |
| **Driver / plugin drop-in** | Same app, new backend module | Patroni `AbstractDCS` |
| **Library drop-in** | Link different crate, similar API | App switches RocksDB→PedraDB or SQLite→PedraDB |
| **Semantic subset** | Same protocol, fewer features | PG wire with limited SQL |
| **Semantic fork** | Same language, different guarantees | CQL on CP store ≠ Scylla |

Always say which type you mean when promising “replace X.”

---

## Suggested plug order (grail realism)

```
1. Library plug     App → PedraDB              (justify kernel)
2. Plugin plug      Patroni → DCS plugin       (etcd role, smaller than full etcd wire)
3. Wire plug        etcd API → same DCS        (true “where etcd was”)
4. Wire plug        MySQL or PG → SQL+dist KV  (TiDB/CRDB class — large)
5. Avoid as primary Scylla **CQL/AP** wire; prefer replace **need** via KV+watch gateway
```

---

## One diagram: “where do I point the config?”

| Today | Tomorrow (conceptual) |
|-------|------------------------|
| Patroni `etcd3.hosts: etcd:2379` | `etcd3.hosts: pedra-dcs:2379` **or** `dcs: pedra` plugin |
| App `sqlite3_open("x.db")` | `pedradb::open("x")` or `pedra_sqlite_open` facade |
| App `postgres://…` single primary | `postgres://sql-gateway…` (horizontal SQL product) |
| TiDB `mysql://tidb…` | `mysql://our-sql…` |
| TiKV PD/TiKV endpoints | our-kv endpoints |
| Scylla CQL (customer AP apps) | **Keep Scylla** unless CP-CQL product |
| Scylla for routes / orchestrator (platform need) | **gRPC gateway** over multi-Raft+PedraDB+watch — see `scylla-need-replacement.md` |

---

## Bottom line

- **Plug** = syntax/driver/wire at the **edge** the ecosystem already uses.  
- **PedraDB** = always the **bottom storage primitive**, never the plug itself (except pure embed).  
- **etcd / Patroni** = DCS product in the middle; PedraDB stores state.  
- **Postgres horizontal / TiDB** = SQL wire + dist stack + PedraDB/node.  
- **Scylla product (CQL/AP)** = not the main-line promise.  
- **Scylla *need* (control-plane metadata + push)** = same stack as TiKV/etcd-class + watches; no drop-in required.

Related: `doctrine-primitives-and-api-layers.md`, `grail-plan-build-databases-on-pedradb.md`, `pedradb-as-dcs-storage-for-patroni.md`, `scylla-need-replacement.md`.
