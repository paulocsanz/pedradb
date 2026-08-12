# MontanhaDb (Montan-HA-DB)

**Status:** product name + architecture north star (living)  
**Updated:** 2026-08-12  
**Normative product contract:** [RFC-0013: MontanhaDb product specification](rfc/0013-montanhadb-product.md) — **implement and validate against that RFC**  
**P0 invariant ↔ test map:** [montanha-invariants-and-tests.md](montanha-invariants-and-tests.md)  
**DST / fault seams (keep determinismo separate):** [dst-seams.md](dst-seams.md)  
**Doctrine vs FDB (read if confused):** [montanha-vs-foundationdb.md](montanha-vs-foundationdb.md)  
**Deep research (market + TiKV/FDB/CRDB/etcd):** [montanhadb-deep-research.md](montanhadb-deep-research.md)

**Pronunciation / reading:**

| Form | Reading |
|------|---------|
| **MontanhaDb** | Primary product name (Portuguese *montanha* = mountain) |
| **Montan-HA-DB** | English expansion: **Montan** + **H**igh **A**vailability **DB** |
| **Montanha** | Short form in speech / ops |

**Relationship to this repo:**

| Name | Role |
|------|------|
| **PedraDB** | Local storage **kernel** — ordered KV + multi-key ACID, one process, embeddable (RocksDB’s *role* + TX) |
| **MontanhaDb** | **Distributed HA product family** on PedraDB — **FoundationDB-shaped** (TX substrate + layers), not “etcd product” and not “TiKV clone” as identity |

```text
┌─────────────────────────────────────────────────────────────┐
│  Layers: DCS · Live · SQL · agents / Patroni-shaped HA      │  ← FDB-style layers
├─────────────────────────────────────────────────────────────┤
│  Montanha-Store  (distributed ordered KV + TX substrate)    │  ← the mountain base
├─────────────────────────────────────────────────────────────┤
│  PedraDB  (local kernel per peer: embed KV + multi-key TX)  │
└─────────────────────────────────────────────────────────────┘
```

**Order of construction (normative):**  
1) PedraDB (local FDB-inspired kernel) → 2) **Montanha-Store** (distributed substrate) → 3) **layers** (DCS, SQL, Live).  

**Doctrine:** more **FoundationDB** than TiKV at the *product* layer  
([montanha-vs-foundationdb.md](montanha-vs-foundationdb.md)).  
Multi-Raft ranges are **plumbing** for replication/scale today — not the brand.

Today’s single-domain Raft+DCS is a **bootstrap**, not the end state.  
Full layering: [montanha-layering-dcs-on-store.md](montanha-layering-dcs-on-store.md).

PedraDB stays a **library**. MontanhaDb is the **system** operators run for high availability and (over time) horizontal scale.

---

## 1. One-sentence product

**MontanhaDb is a FoundationDB-shaped multi-node platform on PedraDB: an ordered transactional KV substrate, layers for coordination and higher models, correct fencing by default — without etcd as product identity.**

---

## 2. Why this name

- **Montanha** — durable, layered, hard to erode; the stack is a mountain of layers, not a kitchen-sink monolith.  
- **Montan-HA-DB** — explicit **HA** in the English reading; not “another embedded toy.”  
- **Db** — product is a **database system** (coordination + data plane path), while the **kernel** remains PedraDB.  
- Distinct from PedraDB so we never confuse **embed library** with **cluster product**.

---

## 3. Positioning vs market

### 3.1 What MontanhaDb is *not*

| Not this | Why |
|----------|-----|
| “A better etcd binary” | Market already has etcd + Kine + Consul; we refuse etcd footguns as identity ([dcs-market-landscape.md](dcs-market-landscape.md)) |
| Multi-master same-key writes | Split-brain; wrong for HA locks |
| PedraDB multi-process on one directory | Forbidden; one PedraDB dir = one process |
| Drop-in TiKV or FDB wire day one | **Philosophy** is FDB-shaped; wire clones are non-goals |
| Claiming FDB production parity | Kinship of *shape*, not years of simulation/ops |

### 3.2 What MontanhaDb *is* (FoundationDB-shaped, correct)

| FDB-like idea | MontanhaDb ideal |
|---------------|------------------|
| Ordered KV + transactions as the core product | **PedraDB** locally; **Montanha-Store** as distributed substrate |
| Everything richer is a **layer** | DCS, SQL, Live, HA agents — libraries / thin services on the store |
| Locks / election = data + TX | create/CAS/watch on keys — **not** “we are etcd” |
| Apps don’t need a second coord database | One substrate; meta keys + user keys (prefixes / ranges under the hood) |
| Honest limits & ops | Typed errors, named read models, anti-etcd footguns |

| Implementation tool (not identity) | Role today |
|-------------------------------------|------------|
| Multi-Raft ranges | Majority-durable replication & scale-out writes in `pedradb-store` MVP |
| Single-domain Raft | Bootstrap / network demos only |

**“Correto e ideal” means:**

1. **Correctness first** — fencing tokens, majority commit, no silent watch gaps.  
2. **Honest consistency** — every API names local vs linearizable vs best-effort live.  
3. **Layering (FDB lesson)** — kernel tiny; distribution + DCS outside pure PedraDB.  
4. **Anti-etcd footguns** — [multi-node-without-etcd-footguns.md](multi-node-without-etcd-footguns.md).  
5. **Live ops** — open leadership sessions ([live-leadership-and-patroni-shaped-ha.md](live-leadership-and-patroni-shaped-ha.md)).  
6. **Grail path** — FDB-shaped substrate; election-as-TX; not a forever-monolithic DCS.

---

## 4. Product surfaces (family)

MontanhaDb is a **family of deployable capabilities**, not a single binary forever.

| Surface | Purpose | Today (in monorepo) |
|---------|---------|---------------------|
| **Montanha-Store** | Horizontal multi-Raft KV (**substrate**) | **`pedradb-store`**: multi-range + majority + `put_batch` (same-range) + **`commit_tx`** (cross-range multi-key 2PC). **Not** full FDB product parity. |
| **Montanha-DCS** | Coord layer **on Store** (TX/CAS) | **`StoreCluster::dcs_create` / `dcs_cas`**; bootstrap single Raft still exists |
| **Montanha-Raft** | Consensus transport (domain / multi-group) | `pedradb-raft`, `pedra-raft-node` |
| **Montanha-Live** | Best-effort leadership streams on meta keys | Design done; hub not shipped |
| **Montanha-HTTP** | Wire for KV/DCS | `pedradb-http` |
| **Montanha-SQL** | Embed SQL subset | `pedradb-sql` (not PG wire) |
| **Montanha-Stream** | Durable log need | `pedradb-stream` |
| **PedraDB** | Embed kernel only | `pedradb-core` |

Public branding can stay **MontanhaDb** for the system; crate names may remain `pedradb-*` until a rename pass (optional, not required for identity).

---

## 5. Architecture north star

### 5.1 Near term (shipped + next)

```text
                    Clients / HA agents / proxies
                              │
              ┌───────────────┼───────────────┐
              │               │               │
         strong CAS      live subscribe    optional HTTP
              │               │               │
              ▼               ▼               ▼
         ┌────────────────────────────────────────┐
         │         MontanhaDb coordination        │
         │  Raft leader proposes DcsCommand       │
         │  Live hub fans out LeaderChanged       │
         └───────────────────┬────────────────────┘
                             │ apply
              ┌──────────────┼──────────────┐
              ▼              ▼              ▼
           Node1          Node2          Node3
           PedraDB        PedraDB        PedraDB
```

- **One Raft group per coordination domain** (e.g. one PG HA cluster).  
- **Truth:** CAS/create + revision.  
- **Live:** open connection, best-effort who-is-leader.

### 5.2 Ideal medium term (FDB-shaped substrate; multi-Raft under the hood ok)

```text
  SQL / app / HA agent
           │
           ▼
  Montanha API (KV + TX + optional SQL layer)
           │
     ┌─────┴─────┐
     │ Scheduler │  (placement, split/merge — PD-like)
     └─────┬─────┘
           │
   multi-Raft regions / ranges
     each range: leader + PedraDB (or shard of PedraDB)
           │
     election / lock = TX on metadata keys
     live hub = subscribe to key/range applies
```

- **Almost all nodes are writers** (for *some* ranges).  
- **No multi-writer same key** without conflict/abort.  
- **App leader election** = transaction, not a second etcd product.

### 5.3 What we refuse

- Shared disk multi-writer.  
- Default linearizable read on every status call.  
- Unbounded coordination history.  
- “Stream alone fences the primary.”  
- Putting OLTP app tables into the coordination raft group.

---

## 6. Correctness model (ideal)

| Operation | Guarantee |
|-----------|-----------|
| Commit of DCS / raft entry | Majority; then apply to PedraDB |
| Acquire / renew leader key | Linearizable via leader propose + CAS semantics |
| `dcs_get` / follower read | Applied (may lag) — documented |
| Leadership stream | Best-effort; cursor; `CursorGone` on gap |
| Primary write acceptance (agent) | Must hold current **fencing rev** |

**Ideal invariant:**  
At most one **fenced** primary for a given lock key in a given epoch (`rev`).  
Live view may lag; stale primaries demote on renew failure.

---

## 7. HA story (Montan-HA)

1. **Data plane HA** (future multi-Raft): replica groups, automatic leader election per range.  
2. **Control / DCS HA:** 3/5 voters, odd only, learners until catch-up.  
3. **Application HA (Patroni-shaped):** agents use Montanha-Coord; proxies use Montanha-Live.  
4. **Ops HA:** typed errors (`NotLeader`, `QuorumLost`, `CursorGone`, future `DiskTooSlow`).

Name **Montan-HA-DB** emphasizes (2)+(3) from day one, (1) as the climb.

---

## 8. Relationship to prior design docs

| Topic | Canonical doc under MontanhaDb |
|-------|-------------------------------|
| Live leadership sessions | [live-leadership-and-patroni-shaped-ha.md](live-leadership-and-patroni-shaped-ha.md) |
| Anti-etcd multi-node rules | [multi-node-without-etcd-footguns.md](multi-node-without-etcd-footguns.md) |
| DCS market reality | [dcs-market-landscape.md](dcs-market-landscape.md) |
| Pedra under DCS | [pedradb-as-dcs-storage-for-patroni.md](pedradb-as-dcs-storage-for-patroni.md) |
| Apply + Raft mechanics | [apply-and-raft.md](apply-and-raft.md) |
| Layer doctrine | [doctrine-primitives-and-api-layers.md](doctrine-primitives-and-api-layers.md) |
| Grail ladder | [grail-plan-build-databases-on-pedradb.md](grail-plan-build-databases-on-pedradb.md) |

**Naming rule going forward:**  
In product prose, say **MontanhaDb** for the distributed HA system; say **PedraDB** for the embeddable kernel.

---

## 9. Implementation map (monorepo today)

| Montanha capability | Code |
|---------------------|------|
| Kernel | `crates/pedradb-core` |
| Fault injection | `crates/pedradb-sim`, `crates/pedradb-dst` |
| Ordered apply / KV façade | `crates/pedradb-apply` |
| Raft + multi-process node | `crates/pedradb-raft`, bin `pedra-raft-node` |
| DCS SM + `DcsCommand` | `crates/pedradb-dcs` |
| HTTP wire | `crates/pedradb-http` |
| SQL subset / stream | `crates/pedradb-sql`, `crates/pedradb-stream` |
| WAL ship replica | `crates/pedradb-replicate` |

Optional later: rename bins/crates to `montanha-*` in a dedicated RFC; **identity is documentation-first**.

---

## 10. Roadmap slices (product, not microtasks)

### M0 — Identity + coord HA (current)

- [x] PedraDB kernel  
- [x] Raft multi-node + persist + failover tests  
- [x] DCS commands on Raft  
- [x] Design: live leadership + anti-etcd doctrine  
- [ ] Ship **Montanha-Live** hub (open sessions)  
- [ ] Branding in CLI/help strings (“MontanhaDb coordination”)

### M1 — Operable HA product

- [ ] Group commit / disk SLO errors on raft path  
- [ ] Membership + cluster id  
- [ ] Agent reference (Patroni-shaped loop)  
- [ ] Proxy example on `LeaderChanged`

### M2 — TiKV-class store

- [ ] Multi-Raft ranges  
- [ ] Placement / split  
- [ ] Election/lock only as TX on metadata  
- [ ] Optional SQL/HTAP layers as products on Montanha-Store

---

## 11. Tagline options (marketing-safe)

- **MontanhaDb — High availability on a correct mountain.**  
- **Montan-HA-DB — Multi-node HA without etcd-shaped traps.**  
- **PedraDB at the base. MontanhaDb at the summit.**

---

## 12. Normative naming checklist

When writing docs or APIs:

| Do | Don’t |
|----|--------|
| MontanhaDb for cluster/HA product | Call the kernel “Montanha” |
| PedraDB for embed library | Say PedraDB is multi-node by itself |
| Montan-HA-DB when expanding HA | Invent MontanHA without hyphen when teaching English readers |
| Link to live-leadership + anti-footgun docs | Promise “better etcd” as the pitch |

---

## 13. North-star sentence

**MontanhaDb (Montan-HA-DB) is the high-availability, multi-node system built on PedraDB: FoundationDB-shaped substrate + layers, Patroni-shaped coordination as data, live sessions as best-effort only — correct by construction. Normative contract: [RFC-0013](rfc/0013-montanhadb-product.md).**
