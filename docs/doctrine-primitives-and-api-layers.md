# Doctrine: powerful primitives + API layers only

**Status:** core product doctrine  
**Updated:** 2026-08-12  

---

## One sentence

**PedraDB is a powerful primitive (local ordered KV + multi-key ACID). Everything users recognize as a “database product” is an API layer on top — not more features stuffed into the kernel.**

The distributed HA product family built on that primitive is **[MontanhaDb](montanhadb.md) (Montan-HA-DB)** — TiKV-class ambition, correct layering.

```
┌──────────────────────────────────────────────────────────┐
│  Montanha-DCS · Live · SQL · stream (layers)             │
│  election/lock = TX on keys · open leadership sessions │
├──────────────────────────────────────────────────────────┤
│  Montanha-Store — multi-Raft KV (horizontal primitive)   │
│  many range writers · placement                          │
├──────────────────────────────────────────────────────────┤
│  PedraDB — local kernel                                  │
│  open · begin · get/put/delete/range · commit            │
└──────────────────────────────────────────────────────────┘
```

**DCS is built on the store**, not the other way around.  
See [montanha-layering-dcs-on-store.md](montanha-layering-dcs-on-store.md).

---

## What “só camadas de API em cima de primitivas” means

| Do | Don’t |
|----|--------|
| Keep PedraDB surface **tiny and stable** | Put etcd/Postgres/CQL protocol inside PedraDB |
| Implement Patroni/etcd/**SQL** as **layers** | Grow kernel into “kitchen sink DB” |
| Encode models as **keys + TX** (FDB style) | Special-case every product in the engine |
| Add Raft **beside** PedraDB for multi-node | Multi-process share one PedraDB directory |
| One primitive reused by many products | One-off storage per product (reimplement RocksDB each time) |

This is the **FoundationDB layer concept**, with PedraDB as the **local** primitive (so you also get embed/SQLite-class without a cluster).

---

## Examples

| Product goal | Primitive (PedraDB) | API layer |
|--------------|---------------------|-----------|
| App embed ACID | TX put row + index | App code / tiny schema helper |
| Replace etcd for **Patroni** election | Atomic put leader key, range members | **pedra-dcs**: lease, watch, Raft; optional etcd API or Patroni plugin |
| TiKV-class KV | Local apply / local TX | multi-Raft + distributed TX API |
| Horizontal Postgres | Same | SQL layer + regions + N leaders |
| Scylla-class AP | — | **Not** this primitive’s job (different physics) |

**Patroni nuance (again):** PedraDB does not speak etcd. A DCS layer uses PedraDB as **storage primitive** the way etcd uses bbolt; Patroni talks to the DCS layer.

---

## Why this is the grail strategy

1. **One hardened kernel** → many products (coord, KV, SQL, embed).  
2. **Correctness of indexes/models** comes from **TX on the primitive**, not from each API reimplementing consistency.  
3. **Small surface** → less sled-style never-done; less fjall-feature race.  
4. **Horizontal scale** is a **layer** (Raft + regions), not a fork of the storage engine.

---

## What must be true of the primitive

For layers to be “só API,” PedraDB must already be strong enough:

- Ordered keys + range (layouts, listings)  
- Multi-key atomic commit (leader CAS, row+index, config+version)  
- Durable apply under a defined policy  
- Single-process, embeddable  
- Boring, documented defaults  

If the primitive is weak, layers reintroduce storage bugs. If the primitive grows protocols, layers never form.

---

## Delivery implication

| Horizon | Work |
|---------|------|
| **Now** | Make the **primitive** real (P0 TX + durability) |
| **Next** | One **API layer** that proves the thesis (e.g. Patroni DCS or embed index kit) |
| **Later** | More layers (distributed KV, SQL) reusing the same PedraDB |

Never: implement etcd/Postgres wire **inside** `pedradb-core`.

---

## Relation to other docs

| Doc | Role |
|-----|------|
| RFC-0001 | What the primitive is |
| grail-plan | Which products/layers on top |
| pedradb-as-dcs-storage-for-patroni | DCS/Patroni layer detail |
| positioning | Why a tiny primitive justifies use |

---

## Syntax on top of deep protocols (another layer, not the kernel)

If the **semantic protocols** are solid (DCS ops, distributed TX KV, relational
catalog + execution), a **syntax / wire layer** is mostly translation:

```
┌─────────────────────────────────────────────────────────────┐
│  L4  Syntax / wire                                          │
│  SQL text · Postgres/MySQL wire · etcd gRPC · CQL · JSON   │
│  “só parse + mapear pra ops do protocolo de baixo”         │
├─────────────────────────────────────────────────────────────┤
│  L3  Deep protocol (semantic API)                           │
│  take_leader · lease · watch · BeginTX · Get/Put range     │
│  Insert row · secondary index maintain · DDL ops             │
│  stable, versioned, language-agnostic                        │
├─────────────────────────────────────────────────────────────┤
│  L2  Distribution (if any)                                  │
│  Raft · regions · 2PC                                        │
├─────────────────────────────────────────────────────────────┤
│  L1  PedraDB primitive                                      │
│  ordered KV + multi-key ACID                                 │
└─────────────────────────────────────────────────────────────┘
```

| Layer | Owns | Example |
|-------|------|---------|
| **Syntax** | Parsers, wire codecs, error codes for clients | `SELECT …`, Postgres startup packet, etcd `Watch` RPC |
| **Deep protocol** | Real meaning: elections, TX, schema | `AttemptAcquireLeader`, `Commit(writes)`, `CreateIndex` |
| **Distribution** | Multi-node agreement | Raft apply |
| **PedraDB** | Bytes + local atomicity | `put`/`range`/`commit` |

### Why this split helps

1. **Many syntaxes, one semantics** — Postgres-wire *and* MySQL-wire *and* a
   custom RPC can all call the same deep SQL/TX protocol.  
2. **Patroni** can use a **small deep DCS protocol** first; etcd gRPC syntax
   later is “just” another codec on the same ops (`LeaseGrant`, `Txn`, …).  
3. **Testing** targets the deep protocol (correctness); syntax tests are
   compatibility suites.  
4. **PedraDB never sees SQL or etcd** — only the deep layer’s encoded keys/TX.

### What not to do

| Mistake | Why |
|---------|-----|
| Parse SQL inside PedraDB | Kernel bloat; freezes storage to one product |
| Invent wire format before deep ops are stable | Rewrite codecs forever |
| Duplicate semantics per syntax | Two “almost leaders” for etcd vs Patroni plugin |

### Order of build

```
1. Primitive (PedraDB)     — must work
2. Deep protocol           — DCS ops / KV TX / relational ops (as needed)
3. One client (library)    — apps call deep protocol directly
4. Syntax/wire             — optional sugar for ecosystem compatibility
```

**Example Patroni path:**

```
Path B (faster):  Patroni plugin → deep DCS API → Raft → PedraDB
Path A (later):   Patroni etcd client → etcd syntax layer → same deep DCS API → …
```

Same deep protocol; syntax is optional.

---

## Bottom line

Yes: **powerful primitive + API layers only.**  
PedraDB = primitive.  
**Deep protocols** = product semantics (DCS, TX KV, SQL ops).  
**Syntax** = thin mapping (SQL text, etcd gRPC, PG wire) on those protocols.  
etcd-for-Patroni, horizontal Postgres, TiKV-class = layers — never protocols inside the kernel.  
That’s the whole architecture.
