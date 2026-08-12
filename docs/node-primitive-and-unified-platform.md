# PedraDB as node storage primitive → unified multi-product platform

**Status:** living product doctrine + architecture target  
**Updated:** 2026-08-12  
**Audience:** anyone shaping core APIs, Montanha, layers (SQL, stream, HTAP), or “one DB that does everything” ambition  

**Complements:**

| Doc | Role |
|-----|------|
| [`doctrine-primitives-and-api-layers.md`](doctrine-primitives-and-api-layers.md) | Kernel tiny; products = layers |
| [`positioning.md`](positioning.md) | Justify use first; order of proof |
| [`performance-ceiling-option-preservation-and-sled-layer.md`](performance-ceiling-option-preservation-and-sled-layer.md) | Amp ceilings; anti dual-primary B-tree |
| [`grail-plan-build-databases-on-pedradb.md`](grail-plan-build-databases-on-pedradb.md) | Build DBs on the kernel |
| [`rfc/0010-dbs-on-top.md`](rfc/0010-dbs-on-top.md) | Product slices on core |
| [`rfc/0016-pedradb-production-robustness.md`](rfc/0016-pedradb-production-robustness.md) | Local kernel production bar |
| [`rfc/0017-montanha-fdb-class-substrate.md`](rfc/0017-montanha-fdb-class-substrate.md) | Multi-node / multi-Raft |
| [`montanhadb.md`](montanhadb.md) | HA product family |
| [`htap-storage-primitives-and-research.md`](htap-storage-primitives-and-research.md) | **Canonical HTAP:** triangle (layout × freshness × isolation), Zhang/Li architectures, LASER/PolarDB/ByteHTAP/HaSiS/PIM, primaries in `references/` |

---

## 1. Primary product objective (north star)

> **Be a Postgres-class system at the high level that replaces the *need* for Scylla + ClickHouse + NATS in one platform** — same product, automatic path selection, multi-primary multi-region, without operating three clusters.

| Face the user sees | What we actually replace (the *need*) | Path underneath |
|--------------------|----------------------------------------|-----------------|
| **Postgres** | Operational SQL, TX, indexes, product tables | SQL layer → **OLTP** Pedra (write-optimized primaries) |
| **Scylla** | Horizontal control-plane / high-QPS shared KV + push | Montanha multi-Raft + any-node accept + watch ([scylla-need](scylla-need-replacement.md)) |
| **ClickHouse** | Logs, telemetry, wide scan / agg | **OLAP RO** projection / learner ([HTAP](htap-storage-primitives-and-research.md)) |
| **NATS / JetStream** | Durable streams, consumers, fanout | Stream layer on log + keys (`pedradb-stream` → product) |

**Ergonomics:** one system, one catalog (or one control plane).  
`orders` → TX primary path; `events` / logs → OLAP path; routes/leases → CP KV + watch; subjects/streams → stream path — **without** “also install Scylla and CH and NATS.”

**Not the claim:** wire-compatible Postgres day one, CQL drop-in, or CH binary clone.  
**The claim:** *one high-level product* that removes the operational need for those four roles, with **honest paths** (CP ranges, named freshness, any-node accept).

### 1.1 Pedra’s job inside that objective

**PedraDB is the storage primitive on every node** — durable ordered KV + multi-key ACID + shippable log — so every path above shares **one SoR physics** per range leader.

**Montanha** turns **N nodes** into multi-primary multi-region: many range leaders, RO promote, OLAP learners, any-node front door.

```text
  Unified product face (future): SQL + streams + KV + HTAP + DCS …
        │  layers / planners / protocols
        ▼
  Montanha — multi-Raft ranges · N writers · placement · HA
        │  each range applies to local Pedra
        ▼
  PedraDB — per-node primitive (this doc’s focus)
        ordered keys · versions · multi-key TX · durable log
```

This is **not** “one on-disk layout is asymptotically optimal for every amp.”  
It is **one source of truth and change log per node**, plus **projections and layers**, plus **horizontal composition**.

---

## 2. Doctrine in four lines

```text
1. ONE SoR per node: ordered keys × versions × multi-key TX × durable apply-log
2. value = inline | ref(blob) — large payloads without forcing every layout
3. N materializations (LSM, vlog, cache, indexes, OLAP RO) — never a second truth
4. Horizontal: many range leaders (Montanha), not multi-process sharing one Pedra dir
```

| Do | Don’t |
|----|--------|
| Keep Pedra surface **necessary and compact** | Stuff ClickHouse/Postgres/Scylla protocols into core |
| Encode products as **keys + TX + log apply** | Dual durable B-tree + LSM primary in one MANIFEST |
| OLAP = **RO replica / projection** of OLTP log | Dual-write app into two stores |
| Multi-leader via **ranges / Raft** | “Active-active same key without consensus” |
| Measure p99 with honest sync labels | Claim Redis durable p99 + warehouse scan free |

---

## 3. The primitive (necessary, flexible, compact)

### 3.1 Core model

| Element | Meaning |
|---------|---------|
| **User key** | Ordered bytes (layouts, indexes, streams = conventions) |
| **Sequence** | Monotonic versions (MVCC, snapshots, apply order) |
| **Value** | Inline bytes **or** `value_ref` → blob/vlog |
| **TX** | Multi-key all-or-nothing commit |
| **Log** | Append-only, crash-recoverable, **shippable** (CDC / replica / OLAP apply) |

**Public mental surface (target compact set):**

```text
open → begin → get | multi_get | put | delete | delete_range | scan(project) → commit
snapshot / get_at / get_value_range
flush · compact · compact_for_reads · compact_vlog
stats · verify · checkpoint · ship_wal / incremental
```

*(Some items are shipped; others are planned — see §7.)*

### 3.2 Why this is enough to *substitute the storage role* of many systems

| System people know | What they actually need from *storage* | Pedra role |
|--------------------|------------------------------------------|------------|
| **SQLite** | Embed file, TX, crash safety | **Direct:** embed Pedra; layer for SQL subset |
| **Postgres** | Durable relations + indexes + TX | **Engine under SQL layer** (keys = rows/indexes); not wire-compatible PG day one |
| **TiKV** | Local engine under multi-Raft | **Per-node apply target** (Montanha-Store) |
| **FoundationDB** | Ordered TX KV (+ layers) | **Local analog** of the storage idea; distribution = Montanha |
| **Redis** | Fast KV (often durable optional) | **Durable ordered KV**; pure in-memory cache is RAM policy/layer, not SoR lie |
| **NATS / JetStream** | Durable stream + cursors | **Keys + log + stream layer** (`pedradb-stream`) |
| **Scylla / Cassandra** | Wide-column, AP multi-master | **Not the same consistency model** — operational metadata / CP paths yes; full AP LWW product is a *different* product choice |
| **ClickHouse / StarRocks** | Analytical scan/agg | **SoR + ship → OLAP RO projection** (column/lake); not CH clone inside LSM |
| **DuckDB** | Embed analytics | **OLAP RO process or in-process engine** reading projection / export; Pedra stays SoR |
| **RocksDB / Pebble** | Embed LSM | **Same family**; Pedra adds **multi-key TX in core** |

**Substitute complete** means: *that system’s job can be implemented correctly on this primitive + layers*, not “byte-compatible clone on day one.”

### 3.3 Write-leaning LSM — does it exclude products?

LSM **privileges sequential write + versioned update**. Reads are made competitive with **bloom, bounds, lazy blocks, cache, compact_for_reads, multi_get**, not by pretending to be a B-tree.

| Workload | Problem if naive | Fit on Pedra |
|----------|------------------|--------------|
| Read-point heavy, write rare | Read amp | multi_get + cache + compact_for_reads |
| Large row / doc updates | Version + compact amp | vlog + get_value_range + compact_vlog |
| Huge range without prefix | Multi-level merge | scan + project; or OLAP path |
| Full-table analytics | LSM ≠ warehouse | OLAP RO / export — **not** hot OLTP path |
| “Postgres heap assumptions” | Wrong access model | SQL layer models **indexes as keys** |

**No second engine in core** for these. **Projections + layers.** Second durable engine only as an **explicit other product**, never dual-write SoR.

---

## 4. Materializations on one node (OLTP / OLAP / cache / indexes)

HTAP is **not** “one engine 100% on both loads, instant freshness, zero extra I/O.”  
It is a **triangle** (layout · freshness · isolation); pure software picks a point. Full map + papers:  
[`htap-storage-primitives-and-research.md`](htap-storage-primitives-and-research.md).

Default product point for this platform:

| Vertex | Choice |
|--------|--------|
| Layout | Row/LSM (+ vlog) on OLTP; **column or external AP** as RO projection |
| Freshness | Named — default **`Applied`** (catch-up / near-sync), not silent dual fsync |
| Isolation | Learner / separate apply (+ cache quotas if same container) |

Three honest shapes (research §10.3): **composed CDC**, **TiFlash-style learner**, **LASER-in-kernel only if proven**.

### 4.1 Modes

| Mode | Mutability | Physical preference | Fed by |
|------|------------|---------------------|--------|
| **OLTP** | R/W | LSM + WAL + vlog | Client commits |
| **OLAP RO** | Apply-only | Column fragments / dense scan layout / external engine | **Same log** (ship/apply) |
| **Cache** | Ephemeral | Row/block cache (RAM; optional disk LRU) | Reads |
| **Secondary indexes** | Via TX | Prefer **prefix keys in LSM**; optional side index dir | Same commits |

```text
  writes ──► OLTP (Pedra SoR) ──log/apply──► OLAP RO
                │                    │
                └── shared process/container optional
                    shared block cache with quotas
```

### 4.2 Same container, reuse storage carefully

**Yes:** one process/container; shared volume; shared **log**; shared **block cache with quotas**; indexes as keys or `olap/idx/`.

**No:** app dual-write to OLTP and OLAP; “same SST file is also columnar”; AP query starving OLTP cache without quotas.

| Resource | Share? | Rule |
|----------|--------|------|
| WAL / sequence | Yes | One truth |
| SST files | OLTP owns | OLAP does not mutate |
| Block cache | Yes + **quotas** | Protect TX working set |
| CPU | Prioritize | commit/apply > AP query |
| Column chunks | Separate dir | Projection cost — see §4.3 |

### 4.3 Storage duplication (honest)

OLAP RO that is good at wide scan **materializes again** (often columnar). That is the standard HTAP trade:

| Approach | Disk | Analytics power |
|----------|------|-----------------|
| HTAP light (scan project only) | ~1× | Limited |
| Indexes + cache | ~1× + idx | Medium |
| Full OLAP projection | ~1× + column (often compressed &lt; 1× row alone) | Strong |

**Duplicating projection ≠ duplicating truth.**  
Default product sync policy: **`Applied`** (OLAP has seen seq) with low lag; **`Durable`** on both stores = opt-in (costs commit p99).

### 4.4 Commit coupling (OLTP ↔ OLAP)

```text
commit(opts: { olap: None | Applied | Durable })
```

| `olap` | Guarantee | Cost |
|--------|-----------|------|
| `None` | Fastest; lag allowed | Lowest |
| `Applied` | After Ok, OLAP serves this seq (**recommended default** for “near-sync HTAP”) | Apply CPU; not 2× fsync |
| `Durable` | OLAP persisted too | Higher p99 |

---

## 5. Core primitives to finish the “compact flexible” set

Shipped vs planned (living — update when code lands):

| Primitive | Status (approx) | Unlocks |
|-----------|-----------------|---------|
| get / put / delete / TX | ✅ | SQLite/TiKV/FDB-local class |
| scan / range_limited | ✅ | Prefix, listings |
| leveled compact, bloom, lazy blocks, lz4 | ✅ | Read/write amp baseline |
| OCC multi-writer (ConcurrentDb) | ✅ | Concurrent TX |
| vlog spill + **compact_vlog** | ✅ | Large values |
| group commit + dual-mem | ✅ | Write concurrency lab |
| checkpoint / backup / ship_wal | ✅ | Ops + CDC seed |
| **CAS / put_if_absent / put_if_eq** | ✅ RFC-0019 | Leases, IF NOT EXISTS, LWT substitute |
| **commit seq pin** (`put_with`, `apply_batch`, `tx.commit`) | ✅ RFC-0019 | Watch watermark / lag |
| **change feed** (`changes` / `changes_after` + CHANGELOG) | ✅ RFC-0019 | CDC / watch catch-up seam |
| **multi_get / multi_get_at** | ✅ RFC-0019 | Mass point read, SQL index path |
| **scan project** (KeyOnly / Full) | ✅ RFC-0019 (KeyOnly) | Cheap range / jobs |
| **get_value_range** (blob slice) | 🔲 planned | Large row partial read |
| **approximate_sizes** | 🔲 planned | Planner / pagination |
| **compact_for_reads** | ✅ RFC-0019 P2.2 | Read-mostly collapse (latest-only full rewrite) |
| **OLAP RO apply + commit olap=Applied** | 🔲 product/RFC | HTAP path |
| Bench harness vs peers (sync-labeled) | 🔲 RFC-0016 P1.4 | Honest perf claims |

**Filter for every new core API:** necessary? compact? unlocks a *class* of product?

---

## 6. Product face: “Postgres for product tables + ClickHouse for logs” — automatic

This is the **user-visible** shape of the grail (not a second storage engine to configure):

> **Write-optimized primaries (Postgres-class SQL / TX path), N primaries across M regions,  
> RO replicas (promote on failover), OLAP-optimized replicas for scan/logs/telemetry —  
> one place, no “also install ClickHouse”; the system routes reads to the right storage.**

### 6.0 Topology (what “N primaries in M regions” means)

```text
 Region A                         Region B
 ┌─────────────────────┐         ┌─────────────────────┐
 │ Primary (leader)    │         │ Primary (leader)    │
 │ ranges … / a–m      │         │ ranges … / n–z      │
 │ Pedra OLTP (write)  │         │ Pedra OLTP (write)  │
 │                     │         │                     │
 │ RO follower(s)      │         │ RO follower(s)      │
 │  · can become leader│         │  · can become leader│
 │                     │         │                     │
 │ OLAP RO learner(s)  │         │ OLAP RO learner(s)  │
 │  · column / scan    │         │  · column / scan    │
 │  · logs/telemetry   │         │  · logs/telemetry   │
 └─────────────────────┘         └─────────────────────┘
              │ multi-Raft / log · placement · catalog
              ▼
        One SQL / product API
        (planner: TX → primary; heavy scan/agg → OLAP RO)
```

| Role | Accepts client writes? | Storage shape | Failover |
|------|------------------------|---------------|----------|
| **Primary (leader)** | **Yes** (its ranges only) | Pedra LSM + TX — **write-optimized** | — |
| **RO replica (Raft follower)** | No | Same row engine, lag ≈ Raft | **Can become primary** on election |
| **OLAP RO (learner / projection)** | No | Columnar / scan-optimized (or external engine fed by log) | Rebuild/relearn; usually **not** promote to OLTP leader unless dual-role policy is explicit |

**Critical rule (no split brain at the *log*):**  
Under the hood, **each key still has one order of commits** (one Raft group / one apply chain).  
That does **not** mean the *client* must talk only to “the” range leader — see **§6.0.5 Any-node accept** (deep resilience).

### 6.0.5 Any-node accept (deep resilience) — what we *do* want

**Product requirement (user-facing):**  
> *Any healthy node can receive any client message (write or read).  
> If “leader 1” dies, the client (or mesh) keeps writing to “leader 2” / any survivor —  
> without the app knowing ranges. Reads can go to RO / OLAP as usual.*

That is **not** the same as “leader 2 is authoritatively multi-master for leader 1’s keys without coordination.”

| Layer | Role |
|-------|------|
| **Front door (any node)** | Accept connection; auth; enqueue / forward / proxy |
| **Ownership (internal)** | One **sequenced log per key-range** (or per shard); one **apply order** |
| **Failover** | New leader elected for that log; front door **retries to new owner** transparently |

```text
  Client ──write any key──► Node B (any healthy node)
                               │
                               ├─ if B owns the range → Raft propose locally
                               └─ if A owns the range → forward / proxy to owner A
                                      (or to new leader after A fails)
                               │
                               ▼
                         One commit order per key
```

**After Node A (previous owner of range X) dies:**

| Step | What happens |
|------|----------------|
| 1 | Client still hits **any** live node (B, C, …) |
| 2 | Cluster elects **new leader for range X** (often an ex-follower of X) |
| 3 | B forwards to that new leader (or becomes it) |
| 4 | Write succeeds — **app did not pick “the right primary”** |

So: **leaders are replaceable and clients are sticky to “the cluster”, not to “primary #1”.**  
Internally there is still **one writer pipeline per range** so two nodes never silently commit conflicting orders for the same key.

**Reads for deep resilience:**

| Read | Where |
|------|--------|
| Point / RYW | Owner primary, or follower with read index / bounded lag |
| During owner outage | Followers of that range (RO); or wait for new leader |
| Analytics / logs | **OLAP RO** for that range (or global projection) — often still up if learner alive |
| “Any node” read | Front door fans out or routes; same as write accept |

**What we refuse (looks resilient, is wrong):**

```text
Node B commits key K while Node A also commits key K
with no shared log / quorum  →  split brain
```

That model is **AP multi-master** (Scylla/Cassandra-class) with LWW/CRDT — a *different* product contract.  
Default Pedra/Montanha grail stays **CP per key-range**, with **any-node front door** for operational resilience.

**Summary one-liner:**  
*Any node can **receive** any message; only the **owner log** (whoever is leader after failover) **orders** the write. Clients never care who the owner is.*

### 6.0.1 Automatic path selection (“just read the right place”)

User / app does **not** configure a second cluster for logs:

| Workload (catalog hints or query shape) | Default path |
|----------------------------------------|--------------|
| Point / short TX / product tables (PK, indexes) | **Primary** (or RO follower if read-only + freshness ok) |
| Failover reads during election | Follower → promote |
| Aggregations, wide scan, **logs / telemetry / events** | **OLAP RO** (same product URI; planner or table property `storage_path=olap`) |
| “I only INSERT and SELECT COUNT by day” | Writes OLTP/log stream; **reads hit OLAP** automatically |

**Ergonomics target:** one connection string / one catalog.  
Tables (or table *classes*) declare intent once:

```text
CREATE TABLE orders (…)           -- product: OLTP primary path
CREATE TABLE events (…)           -- telemetry: write via stream/OLTP log;
                                  -- SELECT defaults to OLAP RO projection
```

No separate “deploy ClickHouse and wire CDC” for the default story — **CDC/learner is the platform**, not the user’s weekend.

*(Implementation: catalog + router; physical OLAP may still be column files or an embedded engine — user does not operate two products.)*

### 6.0.2 Write path vs read path (why it feels like PG + CH)

| | Product tables (PG-shaped) | Logs / telemetry (CH-shaped) |
|--|----------------------------|------------------------------|
| **Write** | TX on primary, sync durable policy | High ingest: append keys / stream; avoid multi-key TX where possible |
| **Primary storage** | Pedra row/LSM (indexes as keys) | Same log SoR; often **not** kept hot on primary longer than needed |
| **Read hot path** | Primary or RO row replica | **OLAP RO** (column, compress, scan) |
| **Freshness** | RYW on primary; RO lag named | OLAP lag named (`Applied` default) |

**Postgres optimized for writes on primaries** = Pedra group commit + dual-mem + sync contract + SQL layer that does **not** plan full-table scans on the primary by default.  
**ClickHouse in the same place** = OLAP projection of the same commits/events, not a second admin plane.

### 6.0.3 Same container optional

Per node (or per range peer):

- Primary process: Pedra OLTP  
- Optional colocated: Raft follower and/or OLAP learner (**cache quotas** so AP does not starve TP)  
- Or OLAP on separate pods reading the log — still **one product**, not “user configured CH”

### 6.0.4 What we refuse (so the automatic story stays true)

| Anti-pattern | Why |
|--------------|-----|
| Two writers, same key, two regions, no consensus | Split brain |
| User dual-writes `orders` to PG path and CH path | Two truths |
| Primary forced to serve warehouse scans | Destroys write-optimized p99 |
| OLAP learner promoted to OLTP without row engine | Wrong layout for TX |

### 6.1 What “replace the cluster products” requires

| Capability | Mechanism |
|------------|-----------|
| Horizontal write scale | **Many ranges**, each with a **leader** (multi-Raft) — Montanha-Store |
| Multi-region writes | Ranges placed by region; **no** silent multi-master same key |
| Same-region multi-leader | Different key ranges, not same key two leaders |
| RO + promote | Raft followers; election → new primary |
| OLAP RO automatic | Log/learner projection; catalog routes SELECTs |
| Resilience | Majority / failover; DST + soak; not “hope” |
| Cross-range TX | Explicit product (2PC / RAMP / avoid) — document cost |
| Single global SQL face | Layer + catalog + **path router** (TX vs OLAP) |

**Pedra never multi-process-opens one data dir.**  
Each node process owns its engines; replication is **log/Raft**, not shared disk.

### 6.2 Mapping “one DB that is CH + PG + Scylla + Redis + NATS”

| Face | Layer | Storage path |
|------|-------|--------------|
| Postgres-class SQL | SQL + catalog + indexes-as-keys | OLTP Pedra (+ OLAP for heavy agg) |
| ClickHouse / StarRocks-class AP | Planner → OLAP RO / column | Projection from log |
| DuckDB-class embed AP | In-process OLAP engine on projection/export | Same |
| Redis-class | API + optional TTL/lease keys; durability policy explicit | OLTP (or pure cache tier non-SoR) |
| NATS-class | Stream + consumer cursors | Keys + log (`pedradb-stream`) |
| **Scylla *need* (control plane KV + push)** | **In grail line** — multi-Raft + watch + any-node accept | See [`scylla-need-replacement.md`](scylla-need-replacement.md) |
| Scylla **product** (CQL, AP multi-master same key) | **Not** default contract | Optional LWW layer only with eyes open |
| TiKV / FDB-class | Montanha multi-Raft + TX model | Pedra per range |
| SQLite-class | Embed only Pedra (+ tiny SQL) | Local file |

**Ergonomics of “one Postgres with extensions”** = **one query/control plane**, many **execution paths**, one **node primitive**, one **range fabric**.  
**Performance** = path selection + physics, not one magical operator.

### 6.3 Scylla: replace the *need*, not the CQL product

**Intent:** use **Pedra primitives + Montanha + watch + any-node accept** to **retire Scylla** as the shared store for high-speed horizontal control plane (routes, WID→host, orchestrator state, discovery) — same class of job Railway runs on Scylla today.

Full architecture: [`scylla-need-replacement.md`](scylla-need-replacement.md).

| We ship | We do not ship as default |
|---------|---------------------------|
| Horizontal N leaders (by key range) | CQL / Alternator drop-in |
| Sub-second change notify (watch) | AP multi-master same-key LWW as core guarantee |
| CAS / multi-key TX (better than LWT islands) | “Any replica commits without quorum of the key’s log” |
| **Any node receives client ops** (forward to owner) | App must pick “the” contact primary |

**AP multi-master same key** remains a *different* contract (optional LWW layer later).  
**Replacing Scylla *need*** is **on the main line.**

---

## 7. Roadmap sketch (not a schedule — dependency order)

| Phase | Focus | Outcome |
|-------|--------|---------|
| **K — Kernel bar** | RFC-0016 remainder (bench, backup under load); primitives §5 | Node SoR production-credible |
| **L — Layer completeness** | SQL, stream, DCS, map/sled DX; multi_get/project first | Substitute *roles* of SQLite/Redis/NATS/PG-embed |
| **H — HTAP Nível 1** | OLAP RO apply, `olap=Applied`, lag metrics, same-container quotas | CH path for logs/telemetry without second product |
| **M — Montanha P0+** | Multi-process majority, placement, N leaders, RO promote | Multi-primary multi-region (§6.0) |
| **U — Unified face** | One SQL/catalog: product tables → OLTP, events → OLAP by default | “PG + CH same place, automatic” (§6.0.1) |
| **R — Resilience proof** | Cluster DST, multi-region failover, silent_wrong=0 under partition schedules | Trust to run for real |

Update checkboxes via RFCs (0016, 0017, future HTAP/ODCS RFC). Do not mark “unified face” done without path-correctness tests.

---

## 8. Success criteria (capability — not “years of field”)

A claim is allowed only when **function + architecture + measured path + test** exist:

| Claim | Gate |
|-------|------|
| Embed SQLite-class | TX + durable reopen + simple app tests |
| SQL operational | Layer tests: row+index one TX; no half index |
| Multi-get / read path | multi_get tests + optional I/O count vs N×get |
| Large values safe | vlog GC reclaim + mid-GC fault tests |
| HTAP near-sync | commit Applied → OLAP read; lag histogram; rebuild OLAP |
| N leaders | multi-range put under failover; no split brain on same range |
| RO promote | follower becomes leader; writes resume; test election |
| “One product face” | Router tests: product table SELECT → OLTP; events agg → OLAP; no user CH config |
| Multi-region | put to local primary; remote key → redirect or explicit latency |
| Perf peer claim | Sync-labeled bench table (RFC-0016 P1.4) |

**Missing test ⇒ missing claim.** Add the test; don’t defer to “maturity lore.”

---

## 9. Anti-goals (keep the primitive compact)

- Dual primary B-tree + LSM in one MANIFEST  
- Full merge-operator / CF zoo in core day one  
- Warehouse execution inside `pedradb-core`  
- Multi-process shared data directory  
- “As fast as Redis and always sync durable and full-table CH” in one path  
- Silent dual-write OLTP+OLAP  

---

## 10. Summary

| Question | Answer |
|----------|--------|
| What is PedraDB? | **Per-node storage primitive:** ordered TX KV + durable log + projections |
| Substitute for many DBs? | **Yes as the storage/role underneath** — products are layers + Montanha |
| One DB that does PG+CH+Redis+NATS+…? | **Yes as unified face** over **one primitive + paths** (OLTP / OLAP RO / stream / cache policy) |
| Multi-leader multi-region? | **Montanha multi-Raft ranges**, not shared Pedra dir |
| Need parallel engine in core? | **No** for default grail; OLAP = RO projection; other families = other products |
| Flexible yet compact? | **Log + keys + TX + value_ref**; everything else materializes |

```text
Um SoR por nó.
N projeções (incl. OLAP RO).
N líderes por ranges.
Uma cara de produto no futuro.
Zero segunda verdade.
```

---

## 11. Related implementation notes (conversation lock-in)

Recorded here so they are not only chat history:

1. **Ideal primitive** = ordered durable TX change substrate, not one optimal layout for all amps.  
2. **HTAP** = OLTP SoR + OLAP RO replica from log; default **Applied**, Durable opt-in.  
3. **Same container** OK; cache **quotas**; indexes preferably **keys**; column chunks optional.  
4. **Storage duplication** of projections is normal; do not dual-write.  
5. **Core API candidates:** multi_get, scan project, get_value_range, approximate_sizes, compact_for_reads.  
6. **RFC-0016 P0** (vlog GC, stats, soak) is the local robustness bar; **0017** is horizontal.

When these ship, tick §5 and open a dedicated HTAP/ODCS RFC rather than growing this doc into an implementation tracker.
