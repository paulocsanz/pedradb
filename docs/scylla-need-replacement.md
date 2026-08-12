# Replacing the *need* for Scylla (not Scylla the product)

**Status:** architecture reframing  
**Updated:** 2026-08-11  
**Audience:** PedraDB grail plan + anyone comparing us to Railway-style control planes  

This document answers a refined question:

> We do **not** need CQL drop-in, Alternator, or Cassandra multi-master semantics.  
> We need an architecture that **removes the operational and scale need** for a Scylla cluster  
> when the real workload is **high-speed horizontal control plane** (route discovery, DNS,  
> overlay WID→host, orchestrator state) — as in `railway/mono`.

**Related:** `scylladb-architecture.md` (how Scylla works as a product),  
`grail-plan-build-databases-on-pedradb.md` §10 (old “off main line” for AP multi-master),  
`plug-map-replace-incumbents.md`, `upsides-only.md`, `switch-justification-bar.md`.

---

## 1. Two different “Scylla problems”

| Problem | What people mean | PedraDB answer |
|---------|------------------|----------------|
| **A. Scylla product** | CQL apps, any-replica write same key, LWW, repair, Seastar LSM | **Not the main line.** Different CAP contract. |
| **B. Scylla *need*** | Platform uses Scylla as **shared, fast, scale-out KV + push** for networking/orchestration | **In scope as architecture.** Replace with multi-Raft ordered KV + PedraDB + watch/push layer. |

Earlier docs correctly rejected **A**.  
This doc is about **B**: Railway already shows the shape of need; Express plan even lists  
**“Scylla replacement | Needed | Current system strained.”**

You win **B** without becoming Scylla.

---

## 2. What mono actually uses Scylla for

Evidence from `railway/mono` (network-cp, orchestrator, Express plan, stacker networking vision) —  
not a full inventory of every table, but the **roles**.

### 2.1 Network control plane (high-speed horizontal networking)

| Role | What happens today | Why Scylla was chosen (approx.) |
|------|--------------------|----------------------------------|
| **Privnet route store** | Routes (`target`, protocol, endpoints, version) live in Scylla; network-cp writes them | Global cache, high QPS, key-partitioned, multi-writer *services* (not multi-writer same key) |
| **Regional discovery** | Port **8450**, Scylla-backed; **CDC** (and WarpStream as secondary path) for real-time route updates | Sub-second awareness without full-table poll |
| **DNS off Redis** | Vision: `stacker-dnssrv` queries Scylla via regional discovery — **one** external data dependency | Scale + single source of truth for resolution |
| **Overlay control plane (planned)** | Forwarding table **WID → host**; **Scylla subscription** for route updates (~sub-second); data plane stays hot with last-known table if CP dies | Push of tiny control messages at fleet scale |
| **Live migration / promote** | Table update + **ScyllaDB push** so clients retransmit to new host within TCP RTO (~200ms) | Propagation budget is **network-control**, not batch analytics |

**Data plane is not Scylla.** WireGuard / future overlay / BPF do packets.  
Scylla is the **authoritative map** + **change fanout** for that map.

### 2.2 Orchestrator control plane

| Role | What happens today |
|------|--------------------|
| **Shared scheduling state** | Stacker registry, status, assignments, volumes, builders, dynamic config — **all pods stateless**, state in Scylla |
| **Horizontal API/scheduler** | Many replicas read/write Scylla; cache poll ~2s for status |
| **Compare-and-set / “leases”** | LWT + `LocalSerial`: volume leases, assignment insert-if-not-exists, Express idempotency, factory_vm FSM transitions, sandbox volume exclusive hold |
| **FSM commit** | Transition outcome + generation + metadata → one Scylla row |

Again: **not** “customer Cassandra app.” It is **distributed system metadata** with occasional linearizable single-row CAS.

### 2.3 What they are *not* buying from Scylla (for these paths)

- Multi-key serializable transactions across arbitrary keys (LWT is single-partition).  
- SQL.  
- True multi-master concurrent writers on the **same** route/WID with LWW as the product feature.  
- Seastar per se — they want **throughput + scale + ops that don’t fall over**.

For route/WID/assignment keys, **single logical writer per key at a time** is the natural model  
(one network-cp mutation path, one promote, one lease holder). That is **CP-friendly**.

---

## 3. What the *need* reduces to (capabilities)

Strip the product name. The platform needs:

```
1. Durable ordered key-value          (route, WID→host, assignment, lease, status)
2. Horizontal scale-out               (many nodes, many leaders, region/shard by key)
3. Strong-enough single-key / multi-key mutations
   - CAS / exclusive lease / FSM generation
   - preferably multi-key when updating route + secondary indexes
4. Low-latency reads                  (DNS / discovery / scheduler hot path)
5. Sub-second change notification     (CDC-like or watch: push to overlay daemons / DNS caches)
6. Stateless app pods                 (orchestrator, network-cp) over shared store
7. Ops that don’t melt under fleet growth
```

**Scylla satisfies 1–7 today with AP defaults + LWT islands + CDC.**  
**PedraDB + multi-Raft + a watch product** can satisfy 1–7 with **CP physics** and **no CQL**.

---

## 4. Target architecture (replace the need)

```
┌──────────────────────────────────────────────────────────────────────────┐
│  Edge consumers (data plane stays dumb + fast)                           │
│  • overlay daemon: WID → host table                                      │
│  • stacker-dnssrv / regional discovery clients                           │
│  • edge proxy route resolution                                           │
└───────────────────────────────▲──────────────────────────────────────────┘
                                │ watch / push stream (sub-second)
                                │ (gRPC subscribe, log tail, or CDC-like)
┌───────────────────────────────┴──────────────────────────────────────────┐
│  Control-plane product layer  (NOT PedraDB core)                         │
│  • Route service, DNS discovery, overlay CP, orchestrator gateway        │
│  • App-level schemas: /routes/…, /wid/…, /stackers/…, /leases/…          │
│  • Optional: thin CQL or proprietary API if someone insists — not required│
└───────────────────────────────▲──────────────────────────────────────────┘
                                │ KV + TX + range + CAS
┌───────────────────────────────┴──────────────────────────────────────────┐
│  Distribution layer (TiKV / FDB-shaped)                                  │
│  • Multi-Raft regions; key → region → single leader                      │
│  • N writers = N leaders (different keys/ranges in parallel)             │
│  • Apply hook: after commit, emit change event to watch bus              │
└───────────────────────────────▲──────────────────────────────────────────┘
                                │ open / begin / get-put-range / commit
┌───────────────────────────────┴──────────────────────────────────────────┐
│  PedraDB (this repo) — local ACID ordered KV per node                    │
└──────────────────────────────────────────────────────────────────────────┘
```

### 4.1 Key design mapping (mono → PedraDB stack)

| Mono / Scylla today | PedraDB-stack equivalent |
|---------------------|---------------------------|
| Partition key `target`+protocol, endpoints map | Ordered keys `/route/{proto}/{target}` + value blob or subkeys |
| Row version / filter-and-bump | Multi-key TX or CAS on `version` field in one commit |
| LWT `IF NOT EXISTS` / LocalSerial | **Native:** compare-and-swap or multi-key TX (better than LWT) |
| CDC on route table | **Watch / apply-stream** from region leader after Raft commit |
| Regional discovery cache | Local in-process cache + watch invalidation (same as today, different backend) |
| Overlay Scylla subscription | Same watch product; consumer updates forwarding table only |
| Orchestrator full-table status scan | Range prefix `/stackers/` or secondary index keyspace; still scale by sharding |
| Poll every 2s status cache | Prefer **watch** or short poll on revision; 2s poll is a product choice, not physics |

### 4.2 Why this is *not* multi-master Scylla

| Property | Scylla main path | This architecture |
|----------|------------------|-------------------|
| Same key concurrent writers | Allowed; LWW / timestamps | **One region leader**; conflicts = reject / retry |
| Multi-key atomicity | No (except rare multi-partition LWT pain) | **Yes** on PedraDB (+ 2PC only if keys span regions) |
| Change feed | CDC product feature | Explicit layer on Raft apply (or log shipping) |
| Availability under partition | Prefer AP write | Prefer **CP** for control correctness (stale route is worse than brief unavailability of promote) |
| Seastar / CQL | Core identity | Irrelevant |

For **networking control**, CP is usually the right default: a split-brain WID→host map  
drops or blackholes traffic. Sub-second **correct** push beats available **wrong** write.

### 4.3 Where horizontal speed still comes from

You do **not** need multi-master on one key to scale:

- **Different routes / WIDs / stackers** hash to **different regions** → parallel leaders → aggregate QPS.  
- **Group commit + PedraDB** on the leader node for local durability.  
- **Watch fanout** is a messaging problem (N clients, batched invalidations), not an LSM multi-master problem.  
- **Read replicas** (optional later): follower reads with bounded staleness for DNS if proven safe.

That is the same scale-out story as TiKV/FDB for metadata-heavy platforms — and what Express  
already wants: “Scylla replacement” under strain, not “more CQL.”

---

## 5. Capability recipe (layers only)

Doctrine: PedraDB stays local. Everything else is a product layer.

| Layer | Delivers | Blocks Scylla need? |
|-------|----------|---------------------|
| **L1 PedraDB** | Durable ordered KV + multi-key ACID + optional sync/group commit | Local correctness substrate |
| **L2 multi-Raft + PD** | Shard, elect, rebalance, single-writer-per-key range | Horizontal store like “Scylla as shared DB” |
| **L3 watch / change log** | Subscribe by key prefix; sub-second notify | Replaces CDC subscription role |
| **L3 CAS / leases** | Generation, IF-NOT-EXISTS, exclusive holds | Replaces LWT islands — with stronger multi-key option |
| **L4 network-cp-shaped service** | Routes, DNS discovery, overlay CP API | App leaves Scylla contact points |
| **L4 orchestrator store gateway** | Stackers, assignments, FSM rows | App leaves Scylla for scheduling state |

**No CQL required.** Wire can be gRPC/protobuf (what network-cp and orchestrator already speak).

---

## 6. Latency story (sub-second is a product SLO, not Scylla magic)

Express promote path (conceptual):

```
t≈0       write WID→host (or parent WID → fork host) in distributed KV
t≈Raft    commit on region leader (+ followers for durability)
t≈push    watch bus notifies overlay daemons / discovery
t≈200ms   TCP RTO retransmit hits new host
```

Budget that must stay **small and predictable**:

1. Raft RTT for the **region** (local AZ preferred).  
2. Apply → notify fanout (push, not 2s poll).  
3. Daemon map update (sub-ms in userspace).

Scylla CDC is one implementation of (2).  
**Raft-apply hooks + dedicated notification service** is another — often *simpler* for CP metadata  
than running a full wide-column cluster for maps of WIDs.

If a region is multi-AZ, Raft latency grows — same as any CP store; **place leaders near writers**.

---

## 7. What we deliberately do *not* claim

| Claim | Verdict |
|-------|---------|
| “PedraDB is a Scylla drop-in” | **False** |
| “CQL apps migrate unchanged” | **False** (and not required) |
| “AP multi-master same-key QPS like Cassandra defaults” | **Out of main line** |
| “We replace Seastar LSM performance on pure wide-column analytics” | **Not the goal** |
| “We can replace Railway’s *need* for Scylla for routes/orchestration with multi-Raft+PedraDB+watch” | **True as architecture** — product work is large |
| “Day-1 PedraDB alone replaces the cluster” | **False** — L2/L3 required |

---

## 8. Switch justification (when mono-like systems leave Scylla)

| Today’s pain (typical) | Bar for switch |
|------------------------|----------------|
| Cluster ops / strain / repair / capacity (Express: “strained”) | Stable multi-Raft + clear runbooks |
| LWT as only linearizable island; inconsistent with plain writes | Uniform TX/CAS model |
| CDC + Kafka dual path complexity | One watch pipeline from apply |
| Redis + Scylla dual dependency for DNS/routes | Single store + watches |
| Multi-key correctness gaps (route + index, FSM + lookups) | PedraDB multi-key TX |
| Want same substrate as DCS/SQL later | One kernel under control plane + other products |

**Not a justification:** “we want CQL compatibility.”  
**Is a justification:** “control plane needs scale-out ordered KV + push + CAS, and Scylla is the wrong physics/ops tradeoff.”

---

## 9. Relationship to earlier “Scylla off main line”

| Statement | Still true? |
|-----------|-------------|
| Don’t design PedraDB kernel for AP multi-master | **Yes** |
| Don’t promise true Scylla product replace | **Yes** |
| Scylla is irrelevant to the grail | **No** — **need** is relevant; product is not |
| Grail has no high-QPS horizontal metadata story | **No** — multi-Raft + watches **is** that story |
| Networking control plane is a separate universe from TiKV/etcd-class | **No** — same substrate class as etcd/TiKV metadata |

**One-line update:**  
**Off main line = Scylla semantics. On main line = Scylla *jobs* (scale-out CP metadata + fanout) done with CP architecture.**

---

## 10. Suggested key layouts (illustrative)

Not normative schema — shows ordered-KV freeness:

```
/route/dns/{fqdn}                 → endpoints, version, service, env
/route/tcp/{host:port}            → …
/wid/{workload_id}/host           → host overlay endpoint, generation
/stacker/{id}/status              → heartbeat blob
/stacker/{id}/meta                → config
/assignment/deploy/{id}           → placement
/lease/volume/{id}                → holder, expiry  (CAS)
/fsm/factory_vm/{id}              → state, generation, pendingMutation
```

Secondary indexes as extra keys in the **same multi-key TX** (PedraDB strength vs LWT single partition).

---

## 11. Implementation order (honest)

```
P0  PedraDB kernel (WAL → MemTable → TX → SST)     ← this repo
P1  Multi-Raft + range placement (distributed KV) ← product
P2  Watch / change stream on apply                 ← product
P3  Control-plane gateway (routes + leases + FSM)  ← product
P4  Cutover mono-like workloads off Scylla         ← migration
```

PedraDB alone never “kills Scylla.”  
**PedraDB + L2 + L3** kills the **need** for Scylla on this class of system.

---

## 12. Bottom line

1. **Drop-in Scylla is the wrong goal.**  
2. **Mono’s Scylla is mostly: scale-out control-plane KV + CAS islands + CDC push for networking and orchestration.**  
3. That maps cleanly to **PedraDB (local) + multi-Raft (horizontal, single-writer-per-key) + watch layer (sub-second) + service gateways (gRPC).**  
4. Horizontal speed comes from **many leaders on many keys**, not multi-master on one key.  
5. Express already names **Scylla replacement** as a prerequisite under strain — the architecture above is the honest CP answer.

When someone says “but Scylla?”, reply:

> We don’t replace Scylla the database. We replace the **platform need** that made you run Scylla  
> for routes, DNS, overlay maps, and orchestrator state — with a CP stack built on PedraDB.
