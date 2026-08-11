# Architecture: PedraDB is the local primitive (not the multi-node DB)

> **PedraDB does not ship multi-node.** It is the embedded local storage
> (and local TX) engine — the role **RocksDB plays for TiKV** and **Redwood
> plays for FoundationDB**.
>
> A **separate product** (our future FDB/TiKV-class database) will **link
> PedraDB as its per-node store**, the same way TiKV embeds RocksDB on every
> TiKV server. That outer DB owns Raft, PD, networking, cross-node TX.
> PedraDB owns the disk on one machine.

---

## One picture

```
┌─────────────────────────────────────────────────────────────────┐
│  OUR FUTURE DB (separate product — not PedraDB)                 │
│  “tipo TiKV / FDB”                                              │
│                                                                 │
│   multi-Raft · routing · TSO/HLC · 2PC/Parallel Commits · gRPC  │
│                                                                 │
│    ┌──────────┐   ┌──────────┐   ┌──────────┐                   │
│    │  Node A  │   │  Node B  │   │  Node C  │                   │
│    │ ┌──────┐ │   │ ┌──────┐ │   │ ┌──────┐ │                   │
│    │ │Pedra │ │   │ │Pedra │ │   │ │Pedra │ │  ← one PedraDB    │
│    │ │ DB   │ │   │ │ DB   │ │   │ │ DB   │ │    instance per   │
│    │ │(local)│ │   │ │(local)│ │   │ │(local)│ │    node/process │
│    │ └──────┘ │   │ └──────┘ │   │ └──────┘ │                   │
│    └──────────┘   └──────────┘   └──────────┘                   │
└─────────────────────────────────────────────────────────────────┘

PedraDB alone (this repo, this product):

┌──────────────────────────────────────┐
│  App or outer DB process             │
│    └─ pedradb (library)              │  single process, single machine
│         WAL · MemTable · SST · TX    │  no cluster, no Raft, no PD
└──────────────────────────────────────┘
```

| Product | Scope | Analogy |
|---------|--------|---------|
| **PedraDB** (this project) | **Local only** — library, one process, one disk | **RocksDB** inside TiKV; **Redwood** inside FDB |
| **Outer DB** (future, other name) | Multi-node TX KV (or SQL) | **TiKV** or **FDB** as a product |
| **Layers** on the outer DB | SQL, doc, graph… | TiDB on TiKV; Record Layer on FDB |

**PedraDB is never “the cluster.”** The cluster **uses** PedraDB.

---

## Why this split

| If PedraDB tried to be TiKV/FDB | If PedraDB stays local (correct) |
|--------------------------------|----------------------------------|
| Mixes engine + consensus + product | Clear job: best local ordered KV (+ local TX) |
| Forces Raft/PD into the same roadmap as SST format | Outer DB can be designed later on a stable store |
| Competes with TiKV as a full distributed product | Competes with **RocksDB/Pebble** as the substrate |
| Harder to embed in random apps | Any app or any cluster can `use pedradb` |

TiKV’s lesson: they **wrapped RocksDB** and then spent years on TX + multi-Raft
**outside** RocksDB. PedraDB’s job is to be a **better thing to wrap** — so the
outer DB (when we build it) does not inherit RocksDB’s write-amp, and can
optionally get **local ACID** from the library instead of inventing everything
on a mute engine.

---

## What *is* inside PedraDB

Still two **internal** layers, both **local**:

```
┌─────────────────────────────────────────────────────────┐
│  PedraDB (single binary/library — one node)             │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Local transactional API (recommended in core)    │  │
│  │  begin/commit · MVCC · OCC · multi-key ACID       │  │
│  │  = so outer DB / apps don’t reinvent local TX     │  │
│  ├───────────────────────────────────────────────────┤  │
│  │  Local storage engine                             │  │
│  │  WAL · MemTable · SST · value log · compaction    │  │
│  │  LSM + WiscKey + Monkey + Dostoevsky              │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

| Inside PedraDB | Outside PedraDB (other product / later) |
|----------------|----------------------------------------|
| WAL, SST, compaction | Raft / multi-Raft |
| Local get/put/scan | Network protocol, gRPC |
| Local multi-key TX (ACID on one machine) | Cross-node 2PC / Parallel Commits |
| Crash recovery on one disk | PD, TSO, rebalancing, membership |
| `#![forbid(unsafe_code)]` engine | Cluster operator, load balancer |

**Local TX ≠ multi-node.**  
ACID on one process is still “RocksDB-class product surface with TX,” not a
cluster. The outer DB uses that when applying a Raft log entry or serving a
single-Region transaction entirely on one node.

---

## Mapping to the industry

| Role | In TiKV stack | In FDB stack | In our stack |
|------|---------------|--------------|--------------|
| Local engine | RocksDB | Redwood | **PedraDB** |
| Distributed KV product | TiKV | FoundationDB | **Future DB (name TBD)** |
| SQL / model layer | TiDB | Record Layer / apps | Layers on the future DB (or on PedraDB embedded) |

```
TiKV node:     [ TiKV Raft + Percolator ] → [ RocksDB ]
FDB storage:   [ FDB roles / replication ] → [ Redwood ]
Our node:      [ Future DB Raft + TX  ] → [ PedraDB  ]
App embedded:  [ App code             ] → [ PedraDB  ]
```

---

## What we are *not* doing in this repo

- ❌ Multi-Raft / PD / cluster membership as PedraDB features  
- ❌ “PedraDB L2 cluster” as part of the PedraDB product  
- ❌ Requiring PgBouncer or any network pooler for PedraDB  
- ❌ Porting Redwood or wrapping RocksDB as the engine  

Distribution design docs (`distribution-design.md`, etc.) remain **research for
the future outer DB**, not a PedraDB roadmap item. They inform what PedraDB
must **support as a library** (apply batch, iterators, local TX, crash safety),
not what PedraDB **implements as a network service**.

---

## What PedraDB must expose so a TiKV-like DB can use it

The outer DB needs roughly what TiKV needs from RocksDB:

| Capability | Why the outer DB needs it |
|------------|---------------------------|
| Durable write batch / atomic apply | Raft log apply → commit state machine |
| Ordered scan / snapshot read | Reads, compaction of logical state, TX |
| Local multi-key TX or atomic batch | Single-Region TX without distributed 2PC |
| Crash recovery | Node reboot |
| Controlled memory / flush | Backpressure, flow control |
| Stable disk format + versioning | Rolling upgrades of the outer DB |

Building **local TX into PedraDB** means the outer DB can treat “this Region’s
leader commit” as a PedraDB transaction instead of hand-rolling MVCC on raw
SST put/get (the expensive TiKV path).

---

## Roadmap (PedraDB only — all local)

| Slice | What | For outer DB? |
|-------|------|----------------|
| 0 WAL ✅ | Crash-safe log | Apply durability |
| 1 MemTable + InternalKey | In-memory ordered buffer | Buffer before flush |
| 2–3 Local TX API | Multi-key ACID on one node | Single-Region TX / apply |
| 4–6 SST, get/scan, compaction | Durable LSM | Long-term store |
| 7 Version GC | MVCC reclaim | Long-lived snapshots |
| 8–9 Sim + oracle | Trust | Same |

**No slice is “add multi-node to PedraDB.”**

---

## Decision log

| # | Decision | Status |
|---|----------|--------|
| 1 | PedraDB = **local library only** | **Accepted** |
| 2 | Multi-node = **separate product** that embeds PedraDB | **Accepted** |
| 3 | PedraDB role = RocksDB/Redwood, not TiKV/FDB product | **Accepted** |
| 4 | Local LSM (not Redwood port, not RocksDB wrap) | **Accepted** |
| 5 | Local TX in PedraDB (so outer DB isn’t forced to bolt-on from zero) | **Accepted** |
| 6 | Distribution docs = research for outer DB, not PedraDB scope | **Accepted** |

---

## Related

- [`architecture.md`](architecture.md) — mission + delivery slices (all local)  
- [`distribution-design.md`](distribution-design.md) — how an *outer* multi-Raft DB would look (not PedraDB features)  
- [`engine-landscape-and-ideal-path.md`](engine-landscape-and-ideal-path.md) — why this LSM  
