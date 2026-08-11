# What would justify switching *to* us

**Status:** product bar / sales-of-engineering  
**Updated:** 2026-08-11  

Question: for each incumbent, **what guarantees and upsides** would someone need to leave SQLite, Postgres, etcd, TiKV, TiDB, Scylla, fjall, RocksDB, … and use **PedraDB + layers**?

If we can’t name a **strict improvement** (or equal guarantee + cheaper/simpler), the switch doesn’t happen.

---

## 1. Guarantees people already expect (the checklist)

Any “DB” they trust usually promises some subset of:

### 1.1 Correctness / consistency

| Guarantee | Meaning (user language) |
|-----------|-------------------------|
| **Atomicity** | Multi-step update all-or-nothing |
| **Durability** | After commit, survives crash (process and/or power — **must specify which**) |
| **Isolation** | Concurrent clients don’t see half-updates (level: RC / SI / serializable) |
| **Consistency (integrity)** | App invariants hold if each TX preserves them (indexes match rows) |
| **Linearizability / strong read** | Read sees latest committed write (when they care) |
| **No silent corruption** | Checksums, recovery that doesn’t invent data |
| **Stable disk format / upgrade path** | Open after upgrade without data science |

### 1.2 Availability / failure

| Guarantee | Meaning |
|-----------|---------|
| **Process restart recovery** | Reopen → last durable commit present |
| **Node failure (distributed)** | Quorum still serves (CP) or both sides serve (AP) |
| **Bounded failover time** | Election / promote within SLO |
| **Backup / restore** | Point-in-time or consistent snapshot story |

### 1.3 Performance / scale (soft “guarantees”)

| Expectation | Meaning |
|-------------|---------|
| **Latency** | p50/p99 under load |
| **Throughput** | ops/s or TX/s |
| **Horizontal scale** | Add machines → more capacity (with model: N writers = region leaders, etc.) |
| **Predictable degradation** | No multi-minute compaction black holes without knobs/docs |

### 1.4 Security / tenancy

| Guarantee | Meaning |
|-----------|---------|
| **AuthN/AuthZ** | Who can connect / which keys/tables |
| **Encryption in transit / at rest** | TLS, disk crypto |
| **Audit** | Who changed what |
| **Tenant isolation** | Soft (prefix) vs hard (process/cluster) |

### 1.5 Operability

| Guarantee | Meaning |
|-----------|---------|
| **Observability** | Metrics, slow logs, TX abort reasons |
| **Online schema / config** | Change without full downtime (product-dependent) |
| **Ecosystem** | Drivers, ORMs, Patroni, k8s operators, runbooks |

### 1.6 Compatibility

| Guarantee | Meaning |
|-----------|---------|
| **Wire/API drop-in** | Same client, little app change |
| **Semantic compatibility** | Same isolation, errors, edge cases — harder than wire |

---

## 2. Bar by incumbent: what would justify the switch

For each: **must match** (or users refuse) + **must beat** (why switch).

### SQLite / redb / local embed

| Must match | Must beat (upsides that justify switch) |
|------------|----------------------------------------|
| Crash recovery of commits | **Multi-key ACID** clearer/faster for their pattern |
| Single-file or simple deploy | **Write path** better for heavy ingest (LSM) *if* that’s their pain |
| Embed, no server | Same + pure Rust stack, or better concurrent writers later |
| Predictable enough ops | Smaller API than “SQL they don’t need” |

**Won’t switch for:** “we have simulation planned,” “papers.”  
**Will switch for:** correct concurrent multi-key updates + embed + speed on their workload + less pain than SQLite locking or than bolting TX on RocksDB.

**PedraDB alone can compete here** (Rung 0–2).

---

### fjall / RocksDB (embed LSM without / weak multi-key TX)

| Must match | Must beat |
|------------|-----------|
| Durability model they understand | **TX-first** so they stop corrupting indexes |
| Throughput/latency on their hardware | Equal or better on **apples-to-apples** sync policy |
| Pure Rust or acceptable FFI | No C++ **and** multi-key ACID |
| Format stability | Don’t sled |

**Won’t switch for:** more features.  
**Will switch for:** fewer footguns (mandatory TX) + same or better perf + substrate story for their next product.

---

### etcd (and Patroni using etcd)

| Must match | Must beat |
|------------|-----------|
| **Leader election safety** (no two primaries) | Same CP safety |
| **Lease/TTL** semantics good enough for failover SLO | Equal or simpler ops |
| **Watch** wake latency acceptable | Equal or better |
| **CAS / conditional put** | Same |
| Durability of DCS commits under 1-node loss (with RF=3) | Same quorum story |
| Prefer: etcd API **or** working Patroni plugin | **Drop-in or one-line DCS change** |

**Won’t switch for:** “PedraDB is a cool LSM.”  
**Will switch for:** replace etcd **dependency** with platform stack; cheaper ops; one kernel shared with other products; equivalent HA for Postgres via Patroni.

**PedraDB does not plug here** — **pedra-dcs** does; PedraDB must only be a **solid apply store**.

---

### Postgres (single node)

| Must match | Must beat |
|------------|-----------|
| Enough **SQL + TX isolation** for the app | Cost, ops simplicity, or embed///scale path |
| Durability of commits | Same |
| Drivers/ORM mostly work | Less ops than full PG **or** better fit (embed) |
| Backup story | Comparable |

**Won’t switch for:** “we’re horizontal someday.”  
**Will switch for:** (a) they didn’t need full PG, only ACID store; or (b) clear path to **horizontal** without Citus complexity; or (c) cost.

**Bar is high** for wire-compatible replace. Easier bar: **new apps** or greenfield.

---

### Postgres / TiDB-class **horizontal** (N writers, M regions)

| Must match | Must beat |
|------------|-----------|
| **Serializability or documented isolation** under concurrent clients | Same correctness as CRDB/TiDB for their TX |
| **No split-brain** under partition (CP) | Same |
| **N write entry points** (region leaders) with scale-out | Better cost, simpler ops, or better locality |
| Failover of regions | Comparable RTO |
| SQL enough for app (or MySQL/PG wire) | Migration cost &lt; benefit |
| Secondary indexes **consistent** with rows | Same TX story |

**Must beat CRDB/TiDB/Cockroach/Yugabyte on at least one of:**  
price, ops simplicity, performance on *their* workload, embed→distributed continuity, Rust/single-kernel platform, multi-cloud, license.

**Won’t switch for:** architecture diagrams.  
**Will switch for:** measured TCO + migration path + equal guarantees on isolation/durability/HA.

---

### TiKV / FDB (distributed TX KV)

| Must match | Must beat |
|------------|-----------|
| Multi-key **distributed** TX correctness | Same |
| Strong consistency / CP under partition | Same |
| Range scans, ordered keys | Same |
| Horizontal scale, rebalancing | Same or better |
| Operational maturity (eventually) | Or much simpler ops earlier |

**Must beat:**  
local TX in engine (faster single-region path), Rust purity, fewer limits than FDB (5s/10MB/100KB) on **local** path, better write amp, cheaper than TiKV ops, embed **and** distribute on same kernel.

**Won’t switch for:** “we also have multi-Raft.”  
**Will switch for:** clear win on **limits, ops, or single-region latency** while matching correctness.

---

### TiDB (MySQL wire distributed SQL)

| Must match | Must beat |
|------------|-----------|
| MySQL compatibility **enough** for their app | Same |
| Distributed TX + HA | Same |
| Online DDL / ops they rely on | Or acceptable subset |

**Must beat:** TiDB cost/ops/perf/license on *their* load.  
**Bar:** extremely high for drop-in; realistic as long-term platform bet.

---

### Scylla / Cassandra

| Must match for *true* switch | Reality vs our plan |
|------------------------------|---------------------|
| Multi-master same key, tunable CL, repair, CQL | **Our main line does not offer this** |
| Extreme single-partition write QPS, AP bias | Different physics |

**To justify switch to *our* stack:** customer must accept **CP + single-writer-per-key-range** (maybe CQL syntax only).  
**To justify switch to Scylla from us:** opposite.

**No honest “replace Scylla with PedraDB grail”** without a second product line.

---

## 3. Minimum bar for *any* switch (universal)

Someone only moves if **all** of these hold:

1. **Guarantees ≥** what they rely on today (durability class, isolation, HA model) — or they **knowingly** relax and accept risk.  
2. **Upside ≥ migration cost** on at least one axis:  
   - money  
   - ops toil  
   - latency/throughput on *their* workload  
   - correctness footguns removed (e.g. multi-key TX)  
   - platform consolidation (one kernel for DCS+KV+SQL)  
   - scale-out writers (region leaders) they don’t have on single-primary PG  
3. **Plug cost low enough** — wire drop-in, plugin, or acceptable API change.  
4. **Trust** — crash tests, production refs, or internal ownership + backup.  
5. **No blocker** — missing lease/watch/SQL feature they can’t live without.

If we only match guarantees and plug is expensive → **no switch**.  
If we beat guarantees but lose 10× latency they care about → **no switch**.

---

## 4. Upsides that *are* our real switch levers (by design)

Map to what we can actually sell:

| Lever | Justifies switch from… |
|-------|-------------------------|
| **Multi-key ACID embed, tiny API** | Ad-hoc files, RocksDB without TX, half of SQLite misuse |
| **Pure Rust kernel** | C++ RocksDB FFI pain, mixed stacks |
| **Same kernel → DCS + KV + SQL later** | etcd + PG + something else ops sprawl |
| **N writers via M region leaders** | Single-primary Postgres write bottleneck |
| **Single-region local TX fast path** | Dist TX always-on systems for local keys |
| **Fewer FDB hard limits on local path** | FDB when embed/local would suffice |
| **Patroni DCS without operating etcd** | etcd as extra dependency for PG HA only |
| **Layer model** | Rewriting storage per product |

If a customer doesn’t value **any** of these, **don’t pitch**.

---

## 5. Guarantees we must *publish* (so switch is rational)

For each product tier, a one-pager:

### PedraDB (library)

| Guarantee | Spec sketch |
|-----------|-------------|
| Atomic multi-key TX | yes / isolation level |
| Durability on commit | process kill vs power loss; default DataSync |
| Crash recovery | last durable commit visible |
| Concurrency | single-writer or OCC; multi-process **unsupported** |
| Ordering | byte-order ranges |
| Non-goals | network, multi-tenant ACL, SQL |

### pedra-dcs (etcd/Patroni tier)

| Guarantee | Spec sketch |
|-----------|-------------|
| At most one leader key holder (with lease) | quorum Raft |
| Lease expiry → others can acquire | max time bounds |
| Watch latency | p99 target |
| Survive 1 of 3 node loss | yes |
| Durability of DCS TX | Raft log sync policy |

### Distributed KV / SQL tier

| Guarantee | Spec sketch |
|-----------|-------------|
| Isolation level | serializable / SI |
| Commit durability | majority Raft |
| Region failover | RTO |
| N writers meaning | leaders per region |
| What happens under partition | CP: minority unavailable |

Without these published, “switch to us” is faith.

---

## 6. Migration cost vs upside (sanity table)

| From → to | Migration cost | Upside needed |
|-----------|----------------|---------------|
| App map/files → PedraDB | Low | ACID + durability |
| RocksDB → PedraDB | Medium (API) | TX + Rust + perf parity |
| fjall → PedraDB | Medium | TX-first + real win |
| SQLite → PedraDB | Medium–high if SQL | Clear non-SQL benefit or SQL subset |
| etcd → pedra-dcs | Low if wire; med if plugin | Ops/platform win, equal HA |
| Patroni etcd → plugin | Low–med | Equal elections, simpler platform |
| PG primary → horizontal SQL | **Very high** | Scale writers + HA + TCO ≫ migration |
| TiKV → our KV | High | Ops/perf/limits win |
| Scylla → our CP stack | **Rewrite app semantics** | Only if they **want** CP |

---

## 7. What would make *us* fail the switch bar

| Failure | Result |
|---------|--------|
| Weaker durability than they had (silent async) | No trust |
| No multi-key TX while promising “DB kernel” | No reason vs map |
| Multi-process surprise corruption | Instant reject |
| Dist SQL without index consistency | Data bugs |
| Claiming Scylla replace with CP physics | Credibility loss |
| Plug requires full app rewrite with no 10× win | No migration |
| Years of alpha (sled) | Abandoned |

---

## 8. Bottom line

**Any guarantee a serious DB has** falls in: atomicity, durability (specify crash type), isolation, integrity, recovery, HA model, security boundary, ops, compatibility.

**To justify a switch** we need:

1. **Match** the guarantees they actually rely on, and  
2. **Beat** them on a lever they care about (TX embed, N writers, one platform, ops, cost, Rust, fewer footguns), and  
3. **Cheap enough plug** (wire / plugin / library).

PedraDB alone justifies switch mainly in the **embed / RocksDB-without-TX** zone.  
**etcd/Patroni, TiKV, horizontal Postgres** need **layers + dist** with **published HA/TX guarantees** equal to incumbents — upside is platform, scale-out writers, or ops, not “we have an LSM.”

---

## Related

- `upsides-only.md` — what we gain  
- `plan-limitations-and-failure-modes.md` — where we lose  
- `plug-map-replace-incumbents.md` — where we plug  
- RFC-0001 — what the kernel promises  
