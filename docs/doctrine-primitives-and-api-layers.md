# Doctrine: powerful primitives + API layers only

**Status:** core product doctrine  
**Updated:** 2026-08-11  

---

## One sentence

**PedraDB is a powerful primitive (local ordered KV + multi-key ACID). Everything users recognize as a “database product” is an API layer on top — not more features stuffed into the kernel.**

```
┌──────────────────────────────────────────────────────────┐
│  API layers (separate products / crates)                 │
│  Patroni DCS · etcd-like · SQL · document · app schemas  │
│  “só protocolo + semântica de produto”                   │
├──────────────────────────────────────────────────────────┤
│  Optional distribution layer                             │
│  Raft / multi-Raft · placement · cross-node TX           │
│  (still not PedraDB core)                                │
├──────────────────────────────────────────────────────────┤
│  Primitive: PedraDB                                      │
│  open · begin · get/put/delete/range · commit            │
│  durable · ordered · multi-key atomic · one process      │
└──────────────────────────────────────────────────────────┘
```

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

## Bottom line

Yes: **powerful primitive + API layers only.**  
PedraDB = primitive.  
etcd-for-Patroni, horizontal Postgres, TiKV-class = **layers** (and distribution next to them).  
That’s the whole architecture.
