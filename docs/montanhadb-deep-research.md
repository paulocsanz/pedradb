# MontanhaDb deep research: market, TiKV-class design, and etcd reality

**Status:** deep research note (primary sources + synthesis)  
**Updated:** 2026-08-12  
**Product:** [MontanhaDb (Montan-HA-DB)](montanhadb.md)  
**Companion:** [DCS market landscape](dcs-market-landscape.md), [anti-etcd footguns](multi-node-without-etcd-footguns.md), [live leadership](live-leadership-and-patroni-shaped-ha.md)

This document is a **research map** for building MontanhaDb as a **TiKV-class** system that is **architecturally correct** (layering, fencing, multi-Raft path) without inheriting **etcd-as-product** footguns or pretending etcd is the apex of coordination.

---

## 1. Executive synthesis

| Claim | Verdict | Evidence class |
|-------|---------|----------------|
| etcd is the best DCS in the world | **False** as absolute | Fragmented market; Patroni multi-backend; Kine; Consul; Nacos; FDB layers |
| etcd is the K8s control-plane **default** | **True** | Kubernetes docs; stacked/external etcd ops literature |
| TiKV-class = multi-Raft + local engine + placement brain | **True** | TiKV architecture; PingCAP multi-Raft design |
| “All nodes writers” without range leaders | **Unsafe** for linearizable locks | Raft/consensus theory; every serious CP store |
| Election as TX on a general KV | **Best long-term API** | FDB layer model; Chubby sequencer / etcd’s own notes on locks |
| Single Raft group for all metadata | **Right for small CP**; **wrong for data plane scale** | etcd docs “few GB”; TiKV multi-Raft for 100TB+ |
| fsync sensitivity is real and operational | **True** | etcd hardware recs; SRE posts; CNCF incident debug notes |

**MontanhaDb target:**  
**PedraDB** local kernel + **Montanha** multi-node: start with correct **coord HA** (one domain Raft + live sessions); climb to **multi-Raft store** (TiKV-shaped); coordination always **data + fencing**, never a second “etcd product identity.”

---

## 2. Taxonomy of systems (what problem each solves)

### 2.1 Coordination / DCS class (small keyspace, strong consistency)

| System | Engine / consensus | Market role | Weakness for “ideal DCS” |
|--------|-------------------|-------------|---------------------------|
| **etcd** | bbolt + **one** Raft group | K8s default CP store | Scale ceiling; fsync; quota/compact; not multi-DC native |
| **ZooKeeper** | Zab + custom RPC | Legacy Hadoop/Kafka coordination | Ops age; recipes external (Curator); limited language story |
| **Consul** | Raft + gossip; discovery-first | Service discovery, multi-DC, mesh adjacency | KV not optimized for huge key counts (etcd’s own critique) |
| **Nacos** | Config + discovery | Dominant in many CN stacks | Different ecosystem gravity |
| **rqlite** | SQLite + **one** Raft | Simple HA SQL; “no extra services” | Single group; not horizontal data plane |
| **Patroni DCS plugins** | etcd/Consul/ZK/K8s | Proves DCS is **pluggable** | Product is Patroni, not the store |

**Primary source (etcd team, biased but useful):**  
[etcd versus other key-value stores](https://etcd.io/docs/v3.5/learning/why/) — positions etcd vs ZK/Consul/NewSQL; explicitly: use etcd for **metadata / coordination** (few GB); use NewSQL for **TB + SQL**.

### 2.2 Distributed SQL / KV data plane (horizontal)

| System | Local engine | Consensus | Control plane |
|--------|--------------|-----------|---------------|
| **TiKV** | RocksDB | **Multi-Raft** regions | **PD** (embeds **etcd** for PD’s own meta) |
| **CockroachDB** | Pebble/engine | **Multi-Raft** ranges + leases | Gossip + range metadata |
| **FoundationDB** | StorageServers (SQLite→Redwood path) | **Unbundled**: Paxos coordinators, sequencers, shards | Coordinators + ClusterController |
| **Spanner** | Colossus/tablet stack | Paxos per group + TrueTime | Global |

These are **not “better etcd”** — they are **general stores**. Using them for coordination means **election as transaction** on keys, not a separate DCS brand.

### 2.3 “etcd API without etcd process”

| Approach | Mechanism | Why markets use it |
|----------|-----------|-------------------|
| **Kine** | etcd API → SQL (SQLite/PG/MySQL) | k3s; reuse ops knowledge of SQL HA |
| **FDB etcd layers** | etcd API → FDB TX | Experiments + managed “etcd on FDB” (e.g. industry 2025 writings) |
| **PG as K8s datastore** | Via Kine | Zone-tolerant CP without etcd ops |

**Market signal:** clients often need the **Kubernetes etcd contract**, not the **etcd binary**.

---

## 3. etcd deep reality (why it dominates *and* hurts)

### 3.1 Why it dominates

1. **Kubernetes gravity** — apiserver persists to etcd; ecosystem docs assume it.  
2. **Simple mental model** — one log, one leader, watches, leases.  
3. **Good enough envelope** — control plane metadata fits “few GB.”  
4. **CNCF / CoreOS history** — default installs (stacked etcd) remove choice.

### 3.2 Production pain (widely documented)

From etcd **hardware recommendations** and SRE literature:

- Consensus requires **majority durable disk writes**; slow disk → latency → **missed heartbeats → elections → instability**.  
- Docs: *“Fast disks are the most critical factor”*; sequential IOPS guidance; SSD for load.  
- Common production symptoms: `apply request took too long`, WAL fsync latency, **database space exceeded** (default quota ~2GiB class issues), ZFS/slow volume footguns.  
- SRE threshold folklore: fsync p99 ideally **&lt; ~10ms** order of magnitude for healthy clusters (monitoring guides).

**Implication for MontanhaDb:**  
Do **not** market “we fsync harder than etcd.” Market wants **correct durability** + **group commit** + **DiskTooSlow typed failure** + **never couple election timers only to pathological fsync**.

### 3.3 Architectural ceiling (by design)

etcd team (own docs):

- **Single consistent replication group** → efficient for ordered metadata, **cannot shard** for horizontal scale of the keyspace.  
- NewSQL shards → global order needs **extra coordination** (clocks / TSO) → worse for pure metadata-ordering workloads **if** you force TB-scale design onto coordination.

**Ideal Montanha split:**

| Workload | Mechanism |
|----------|-----------|
| Coordination domain (locks, leaders, tiny config) | Small Raft group(s) or TX on metadata ranges — **not** one infinite etcd |
| Application data | Multi-Raft ranges (TiKV path) |

---

## 4. TiKV deep dive (the “TiKV da vida” target)

### 4.1 Layers (facts)

Primary architecture materials: [TiKV architecture](https://tikv.org/docs/), [Deep Dive TiKV](https://tikv.github.io/deep-dive-tikv/), PingCAP [Design and Implementation of Multi-raft](https://www.pingcap.com/blog/design-and-implementation-of-multi-raft/) (2017, still foundational).

```text
Client / TiDB
     │
     ▼
┌─────────────┐     heartbeats / scheduling
│     PD      │◄────────────────────────┐
│ (meta+TSO)  │     embeds etcd for PD  │
└──────┬──────┘                         │
       │ region location                │
       ▼                                │
┌──────────────────────────────────────┐│
│ TiKV nodes                           ││
│  Store → many Regions (ranges)       ││
│  each Region = Raft group            ││
│  local RocksDB (raft log + SM keys)  ││
└──────────────────────────────────────┘│
```

**Key design choices:**

| Choice | Detail | Why |
|--------|--------|-----|
| **Range sharding** | Regions `[start, end)` | Scan locality; split/merge mostly metadata |
| **Multi-Raft** | One Raft group per Region | Scale beyond one group’s throughput/size (goal 100TB+) |
| **Local RocksDB** | Prefixes separate Raft meta vs user data | Single engine, careful key layout |
| **PD** | Placement, unique IDs, TSO (with TiDB) | Central scheduler; HA via **embedded etcd** |
| **Peer lifecycle** | Normal / Applying / Tombstone | Snapshot/apply safety |
| **Lease reads** | Leader lease for local read | Avoid Raft for every read when lease valid |

### 4.2 What TiKV gets right (copy into Montanha ideal)

1. **Separation of local engine vs distributed fabric** (RocksDB ≠ TiKV).  
2. **Multi-Raft as the only way to scale writes** past one leader.  
3. **Range keys** for ordered KV + scan.  
4. **Explicit region epoch** on conf change / split (fencing stale routing).  
5. **Scheduler outside the hot path** (PD), not ad-hoc rebalance in clients.  
6. **Batching of Raft ready handling** (WriteBatch across peers on a store).

### 4.3 What TiKV still inherits that Montanha should *not* romanticize

| Issue | Reality |
|-------|---------|
| **PD depends on etcd** | Control plane of the control plane — same class of CP store |
| **Hot ranges** | Range sharding creates hotspots; PD moves regions — ops still real |
| **Complexity** | Raftstore + snapshot + conf change is years of edge cases |
| **Local engine choice** | RocksDB C++ — Pedra wants **pure Rust PedraDB** as the moral RocksDB |
| **Global TX** | TiKV has its own MVCC/TX story with TiDB; not free |

**Montanha “ideal TiKV”:**  
Same **multi-Raft + placement + local engine** shape; **PedraDB** instead of RocksDB; **coordination doctrine** without forcing every product to be a second etcd; **live leadership sessions** for HA agents; long-term **election-as-TX** on metadata ranges.

---

## 5. CockroachDB: multi-Raft at extreme scale

Sources: Cockroach [Scaling Raft](https://www.cockroachlabs.com/blog/scaling-raft/) (2015), replication layer docs, 2025–2026 **Leader Leases** work.

**Facts:**

- Data split into **ranges**, each a Raft group.  
- A node may join **hundreds of thousands / millions** of groups.  
- Naive per-range heartbeats explode → **MultiRaft** multiplexing (heartbeats aggregated per node pair).  
- **Leaseholder** serves reads; classically lease renewals were Raft writes → CPU storm at scale.  
- Recent work: **Leader Fortification / Leader Leases** to unify Raft leader and leaseholder and cut lease traffic ([Cockroach blog 2026](https://www.cockroachlabs.com/blog/distributed-database-leader-leases/), ACM paper “Scalable Leader Leases…”).

**Lesson for Montanha:**

- Multi-Raft **requires** co-scheduling / shared transport early, or you die at modest region counts.  
- **Leases** are a first-class performance feature, not an afterthought.  
- Read path design (who can serve reads) dominates real clusters.

---

## 6. FoundationDB: unbundled “coordination as data”

Sources: [FDB architecture](https://apple.github.io/foundationdb/architecture.html), public design notes, etcd-on-FDB experiments.

**Shape:**

- **Coordinators** (disk Paxos) → elect **ClusterController**.  
- Recruit roles: Sequencer, DataDistributor, Ratekeeper, many **StorageServers**.  
- Clients talk **transactional ordered KV**; layers (Record Layer, custom etcd layer) sit **above**.

**Why this matters for Montanha:**

| Idea | Montanha takeaway |
|------|-------------------|
| Unbundled roles | Don’t force every node to be “full etcd” |
| Layers | DCS / SQL / docs = layers on Pedra/Montanha KV |
| Election/lock | Implement with **transactions**, not a second product |
| Storage evolution | SQLite → Redwood path shows engine can change under stable TX API |

FDB is the strongest open production argument that **“TX + ordered KV” is the OS**, and etcd-like services are **layers**.

---

## 7. Single-group Raft systems (rqlite, classic etcd/Consul KV)

**rqlite:** SQLite + one Raft; market pitch “HA without the hassle.” Perfect for **small** HA SQL; **not** TiKV-class scale.

**Montanha today** (`pedradb-raft` single domain) is in this class for **coordination**.  
**Montanha ideal store** must leave this class for the **data plane**.

---

## 8. Multi-group Raft libraries (implementation substrate)

| Library | Lang | Note |
|---------|------|------|
| **raft-rs / TiKV raft** | Rust | Battle-tested in TiKV |
| **OpenRaft** | Rust | Databend meta, async-first |
| **Dragonboat** | Go | Explicit multi-group performance focus |
| Custom (Montanha now) | Rust | Educational + TCP; not production multi-Raft |

**Research implication:** building Montanha-Store will either **adopt** a mature multi-Raft library or re-learn MultiRaft multiplexing the hard way (CRDB lesson).

---

## 9. Control plane patterns compared

```text
A) etcd-shaped CP store
   Clients → one Raft group → bbolt/sqlite
   Good: simple order. Bad: scale, fsync coupling, product footguns.

B) TiKV-shaped
   Clients → data multi-Raft
   PD (itself etcd-backed) → placement + TSO
   Good: scale. Bad: two systems; PD complexity.

C) FDB-shaped
   Clients → TX to unbundled cluster
   Coordinators minimal; storage sharded
   Good: TX API; layers. Bad: ops complexity; recovery drama historically.

D) Montanha ideal (synthesis)
   Clients → Montanha API
   Coord domain(s): small Raft or TX on meta ranges
   Data plane: multi-Raft + PedraDB (future)
   Live sessions: best-effort leadership (HA UX)
   Explicit consistency; fencing rev; no etcd identity
```

---

## 10. What “correto e ideal” means as measurable design goals

Derived from the research above + prior Montanha doctrine:

| # | Goal | Anti-pattern |
|---|------|----------------|
| 1 | PedraDB remains **single-process** local primitive | Multi-process shared dir |
| 2 | Data scale via **multi-Raft ranges**, not one group | etcd-for-everything |
| 3 | App HA locks via **CAS/TX + fencing rev** | Lease-only mental model |
| 4 | **Named** consistency (local / live / linearizable) | Silent stale or silent linearizable tax |
| 5 | Live who-is-leader via **open sessions** | Poll DCS forever |
| 6 | Cursor watches; **CursorGone** not silent drop | etcd watch reconnect traps |
| 7 | Group commit + disk SLO errors | Election storm on slow disk only |
| 8 | Short metadata history by default | Unbounded MVCC as product |
| 9 | Odd voters; cluster ID; learners | Even clusters; silent merge |
| 10 | One coordination domain per product cluster | Global mega-CP for all tenants without isolation |

---

## 11. Mapping research → Montanha roadmap

| Research pillar | Near (M0–M1) | Ideal (M2+) |
|-----------------|--------------|-------------|
| TiKV multi-Raft | Single domain Raft for coord | Regions + split/merge + PD-like |
| CRDB MultiRaft multiplexing | N/A until many groups | Shared heartbeat/transport |
| CRDB leases | Leader-only proposes | Leaseholder reads + fortification study |
| FDB layers | DCS as commands on Pedra | Election = TX on Montanha-Store |
| etcd market | Don’t clone etcd product | Optional etcd façade later (Kine-like) |
| Live leadership | Design complete | Hub implementation |
| Pedra local engine | Shipped | Keep pure Rust; compete with RocksDB role |

---

## 12. Source index (for re-audit)

| Topic | Source |
|-------|--------|
| etcd vs others | https://etcd.io/docs/v3.5/learning/why/ |
| etcd hardware / disk | https://etcd.io/docs/latest/op-guide/hardware/ |
| TiKV multi-Raft | https://www.pingcap.com/blog/design-and-implementation-of-multi-raft/ |
| TiKV concepts | https://tikv.org/docs/ |
| PD / etcd embed | TiKV deep dive / PD wiki (PD data in etcd) |
| CRDB Scaling Raft | https://www.cockroachlabs.com/blog/scaling-raft/ |
| CRDB leader leases | https://www.cockroachlabs.com/blog/distributed-database-leader-leases/ |
| FDB architecture | https://apple.github.io/foundationdb/architecture.html |
| Patroni multi-DCS | https://patroni.readthedocs.io/ |
| k3s datastores / Kine | https://docs.k3s.io/datastore |
| Dragonboat | https://github.com/lni/dragonboat |
| rqlite design | https://rqlite.io/docs/design/ |
| etcd fsync incidents | CNCF/SRE writeups; HN “check your disks first”; etcd issue #7700 |

---

## 13. Conclusions for MontanhaDb

1. **etcd is market-default for K8s, not theoretical optimum** for all coordination or data.  
2. **TiKV-class** means **multi-Raft + local engine + placement**, not “one raft etcd with a Rust paint job.”  
3. **PD still using etcd** shows even TiKV didn’t eliminate a small CP store — Montanha should treat **coord domains** as first-class and keep them **small and honest**.  
4. **Cockroach** proves multi-Raft at scale is a **transport/lease engineering** problem.  
5. **FDB** proves the **best API** for locks/election long-term is **transactions on ordered KV**.  
6. **MontanhaDb** = brand for climbing that mountain:  
   - base **PedraDB**,  
   - mid **coord HA + live leadership**,  
   - summit **horizontal store with election-as-TX**.

**North star (research-backed):**  
Build the **correct mountain** — TiKV-shaped data plane, FDB-shaped layering for coordination, etcd only as optional *façade*, never as product soul.
