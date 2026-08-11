# PedraDB as storage under a DCS (Patroni elections) — not etcd protocol in PedraDB

**Status:** design note  
**Updated:** 2026-08-11  

---

## The nuance (what you meant)

| Wrong reading | Right reading |
|---------------|---------------|
| PedraDB speaks **etcd wire protocol** | **No.** PedraDB has no network, no etcd API |
| PedraDB *is* the election system | **No.** Election is a **product on top** |
| We replace etcd by “using PedraDB like a remote DB” | **Almost:** we build a **DCS** (distributed configuration store) whose **on-disk state machine** is PedraDB; Patroni talks to the **DCS**, not to PedraDB |

```
┌─────────────────────────────────────────────────────────┐
│  Patroni (or any HA manager)                            │
│  needs: leader key, TTL, CAS, members, config, watch    │
└───────────────────────────┬─────────────────────────────┘
                            │ DCS API
                            │  (A) etcd v3 API  → drop-in
                            │  (B) Patroni DCS plugin → custom
┌───────────────────────────▼─────────────────────────────┐
│  pedra-dcs / “etcd-like” product  (SEPARATE from PedraDB)│
│  · Raft (or multi-Raft later) for multi-node agreement  │
│  · leases / TTL / watches / revision semantics          │
│  · gRPC or HTTP                                         │
└───────────────────────────┬─────────────────────────────┘
                            │ in-process library calls
┌───────────────────────────▼─────────────────────────────┐
│  PedraDB (this project)                                 │
│  ordered KV + multi-key ACID + durable apply            │
│  NO protocol, NO election logic, NO network             │
└─────────────────────────────────────────────────────────┘
```

**PedraDB = bbolt’s role inside etcd** (local durable ordered state).  
**pedra-dcs = etcd’s role** for Patroni (distributed consensus + DCS semantics).

---

## What Patroni actually needs from a DCS

Patroni does not need “a database.” It needs a **Distributed Configuration Store** for HA of Postgres:

| Concern | Typical etcd usage | Semantics |
|---------|-------------------|-----------|
| **Leader lock** | Key with **lease/TTL**; only holder is primary | Race-free acquire (CAS / create-if-not-exists) |
| **Leader renew** | Refresh lease / update leader key | Failure to renew → others can take over |
| **Members** | Keys per node (conninfo, API URL, state) | Heartbeat / `touch_member` |
| **Cluster config** | JSON config key (versioned) | Conditional update (version / CAS) |
| **Failover / sync state** | Extra keys | Coordinated switchover |
| **Watch** | Watch leader or prefix | Fast reaction when leader key changes |
| **Initialize** | Bootstrap marker | Only one initializer |

Patroni abstracts this as **`AbstractDCS`** (`attempt_to_acquire_leader`, `update_leader`, `touch_member`, `watch`, `set_config_value`, …) with backends: **etcd**, Consul, ZooKeeper, Kubernetes, …

So “substituir etcd no Patroni” means: **implement those DCS operations** in a new backend **or** speak enough **etcd API** that the existing etcd backend works.

---

## Two ways to plug into Patroni

### Path A — etcd wire compatible (drop-in)

```
Patroni ──etcd v3 client──► pedra-dcs (implements etcd API)
                              └── PedraDB
```

| Pros | Cons |
|------|------|
| Patroni config almost unchanged (`etcd3: …`) | Must implement a **chunk of etcd API** (KV, lease, watch, TX/CAS) |
| Other etcd clients may work too | etcd API surface is non-trivial; compatibility tax |

PedraDB still only stores bytes; **pedra-dcs** owns protocol.

### Path B — native Patroni DCS plugin (smaller API)

```
Patroni ──AbstractDCS──► pedra_dcs.py (or Rust side + thin HTTP)
                           └── pedra-dcs Raft+PedraDB
```

| Pros | Cons |
|------|------|
| Only implement what Patroni calls | Must maintain Patroni plugin; not drop-in for other etcd users |
| Can match semantics exactly (TTL, watch) | Ecosystem only Patroni (unless others adopt) |

**For “replace etcd election handling in Patroni”**, Path B is often **less work** than full etcd clone. Path A is better if the grail is “general etcd alternative.”

---

## What PedraDB must provide (storage only)

For pedra-dcs apply path:

| Need | PedraDB |
|------|---------|
| Persist `leader`, `members/*`, `config`, … as keys | `put` / `delete` / `range` |
| Atomic “create leader if absent” | multi-key TX or CAS-capable apply (compare-and-set on one key + side effects) |
| Durable after “DCS commit” | Raft log sync **plus** PedraDB durable apply (or Raft log is SoR and PedraDB is derived) |
| Ordered prefix scan | `range("members/")` |
| Snapshots for recovery | reopen / snapshot seq |

**PedraDB does not implement:**

- Lease TTL timers across nodes  
- Watch streams to clients  
- Leader election algorithm (that’s Raft + DCS logic)  
- Patroni or etcd HTTP/gRPC  

Those live in **pedra-dcs**.

### CAS / election on storage

Classic pattern (same as etcd conceptually):

```
# acquire leader (simplified)
begin
  if get("/leader") is None:
     put("/leader", my_id)
     put("/leader_meta", ...)
     commit
  else:
     abort  # someone else holds it
```

Or single-key compare-and-swap if exposed.

With **Raft**: only the Raft leader proposes; apply is serial → CAS is natural in apply order. PedraDB applies the decided batch.

---

## Nuances specific to Patroni + elections

1. **TTL/lease is the hard part**  
   etcd leases expire cluster-wide. You need a **lease manager** in pedra-dcs (tick, revoke keys), not only PedraDB TTL (local clocks ≠ distributed lease).

2. **Watch latency**  
   Patroni `watch(leader_version, timeout)` wants prompt wake on leader change. pedra-dcs must push apply events to watchers; PedraDB doesn’t push to the network.

3. **Split-brain**  
   Safety comes from **Raft quorum**, not from PedraDB. Three pedra-dcs nodes + PedraDB each; if you run one pedra-dcs, you only moved the SPOF.

4. **Don’t open one PedraDB from three Patroni hosts**  
   Multi-process same path is wrong. Each pedra-dcs node has **its own** PedraDB; Raft replicates the log of state transitions.

5. **Consistency with Postgres**  
   Patroni still runs Postgres HA (replication, slots). DCS only coordinates **who is primary**. PedraDB does not store Postgres data in this recipe.

6. **etcd is small-data**  
   DCS keys are tiny. PedraDB’s LSM is fine; no need for Scylla-scale. Durability/sync and Raft matter more than write amp research.

7. **Competing with etcd for Patroni only**  
   You compete on ops (one platform later), not on day-1 “full etcd.” A **Patroni DCS driver** may ship years before full etcd API.

---

## End-to-end flow: failover

```
1. Primary’s pedra-dcs session fails to renew leader lease
2. Lease expires (DCS logic)
3. Standby’s Patroni attempts acquire_leader (CAS create leader key)
4. Raft commits that mutation on majority
5. Apply on each node → PedraDB puts new leader key
6. Watchers fire → other Patroni instances see new leader
7. New primary promotes Postgres; old primary fences (Patroni logic)
```

PedraDB appears only in step 5 (and recovery of local state).  
**Correctness of election = Raft + lease + CAS**, not the LSM.

---

## Layering checklist

| Component | Repo / crate | Speaks to Patroni? |
|-----------|--------------|----------------------|
| PedraDB | `pedradb` | No |
| Raft + apply | `pedra-dcs` (future) | No (internal) |
| Lease + watch + key layout | `pedra-dcs` | Via API |
| etcd gRPC **or** Patroni plugin | `pedra-dcs` / `patroni-pedra` | **Yes** |
| Patroni | upstream | Yes |

---

## What this means for PedraDB roadmap

| PedraDB P0–P1 | Enough for DCS storage? |
|---------------|-------------------------|
| Multi-key TX or atomic batch apply | Yes — CAS/leader write |
| Durable commit / apply | Yes — under Raft apply |
| Range prefix | Yes — list members |
| Watches in kernel | **No need** |
| etcd protocol | **No need** |
| Multi-node | **No** — dcs product |

Optional later: efficient `compare_and_swap` API sugar (still local).

---

## Relation to “grail”

```
Grail platform:
  PedraDB (storage kernel)
       ├── apps / SQLite-class embed
       ├── pedra-dcs → Patroni elections, k8s-ish locks, service discovery
       ├── multi-Raft KV → TiKV-class
       └── SQL → horizontal Postgres-class
```

Replacing **etcd for Patroni** is an early, high-value **upper product** that **stresses** PedraDB as state-machine storage without requiring SQL or multi-master.

---

## One sentence

**PedraDB does not speak etcd and does not run elections; a separate DCS service uses Raft + PedraDB as the durable ordered store, and Patroni either uses that DCS via a plugin or via etcd-compatible API — so you can replace etcd for leader election without putting any protocol inside PedraDB.**
