# PedraDB positioning: small surface, high speed, absurd build-on potential

> The embedded KV space in Rust is **crowded** (fjall, SurrealKV, redb, heed…).
> PedraDB only wins with a **tight focus**, not a bigger feature list.
>
> **North star:** the smallest durable API that still lets someone build a
> serious database (or app) on top — and make that path **fast**.

---

## One sentence

**PedraDB is a tiny, blazing-fast, pure-Rust library: ordered key-value + ACID transactions on one machine — nothing else — so other systems can treat it as bedrock.**

Not a server. Not a cluster. Not SQL. Not Redis. Not “RocksDB with 400 options.”

---

## The problem with a crowded field

| Temptation | Why it fails |
|------------|--------------|
| Match fjall feature-for-feature | Becomes a worse fjall |
| Match SurrealKV + time-travel + … | Becomes a worse Surreal engine |
| Add multi-node “like FDB” | Becomes a worse TiKV, years of work |
| Add SQL / documents in core | Kills the pillar; layers never form |
| Expose every RocksDB knob | Surface explodes; nobody trusts defaults |

**Crowded markets reward subtraction.**  
FDB’s power is what it **refused** to put in the core. Same rule.

---

## Focus (what we optimize for)

Three constraints, in order:

### 1. Surface area → **tiny**

Public mental model fits on a card:

```text
open(path) -> Db
db.begin() -> Tx
tx.get(k) / tx.put(k,v) / tx.delete(k) / tx.range(a..b)
tx.commit() / tx.abort()
```

Optional later (still small): snapshots, explicit conflict ranges, sync policy.
**Not** in v1 surface: column families zoo, merge operators zoo, SQL, secondary
indexes, network, multi-tenancy product features, admin RPC, changefeeds.

If a feature doesn’t make **layers more correct or simpler**, it doesn’t enter.

### 2. Speed → **default path is fast**

- Hot path: put/get/commit/range with minimal allocation and syscalls.
- Durable when asked; no mandatory “enterprise tax” on every op.
- LSM tuned for **write-heavy substrate** use (ingest + TX), not 200 knobs.
- Measure early (microbenches + sim); refuse features that slow the core path.

“Absurdo de potencial” **sem** ser rápido vira paperware. Speed is table stakes
for anyone choosing an engine under their product.

### 3. Build-on potential → **transactions as the superpower**

The only reason this exists instead of “just use fjall/redb”:

> **Multi-key ACID on an ordered KV, local, in-process**  
> so a layer can update data + indexes in one commit without inventing consensus.

That is FDB’s manifesto, **without** requiring a cluster and **without**
FDB’s 5s/10MB/100KB taxes on the local path.

Everything we build (WAL, MemTable, SST, MVCC, compaction) exists to make
**that API** correct and fast — not to showcase storage algorithms for their own sake.

Research (WiscKey / Monkey / Lazy Leveling) is **implementation**, not product
surface. Users don’t configure “Monkey”; they get fast lookups and lower write amp.

---

## Explicit non-goals (permanent until proven wrong)

| Non-goal | Why |
|----------|-----|
| Multi-node / Raft / PD | Other product embeds PedraDB |
| Network server | Library only |
| Query language / SQL in core | Layer |
| Secondary indexes in core | Layer (using TX) |
| Document / graph / wide-column models | Layer |
| Redis-like data types | Different product |
| Object-store-first (S3) | Different niche (SlateDB/Tonbo) |
| Hundreds of tunables | Small surface + good defaults |
| Compatibility with RocksDB on-disk format | Clean-room; oracle for behavior tests only |
| Being “the next sled” feature demo | Ship a boring, small, fast contract |

---

## Who it is for

| Audience | They use PedraDB to… |
|----------|----------------------|
| **Builders of databases / infra** | Local state machine + TX under their protocol (like RocksDB under TiKV, but with local ACID) |
| **App authors who need a real embed** | Correct multi-key updates without Postgres |
| **Us (later)** | Substrate for a multi-node product — **not** this crate’s job |

| Not for | Use instead |
|---------|-------------|
| Cache / ephemeral | Redis |
| “Just ACID B-tree, ship today” | redb / heed |
| Distributed SQL out of the box | TiDB / CRDB |
| S3 bottomless embed | SlateDB / Tonbo |

---

## Competitive wedge (one paragraph)

fjall = excellent general LSM embed (TX optional).  
SurrealKV = LSM+ACID for Surreal.  
redb = stable B-tree ACID.  

**PedraDB = the smallest possible “database kernel”:** ordered KV + **mandatory** multi-key ACID, local-only, ruthless defaults, designed so the next system you write feels like writing an FDB layer — with in-process speed.

We do not win on checklist length. We win on **ratio: power / surface**.

```
        power to build layers (TX + order + durability)
       ─────────────────────────────────────────────
              size of API + ops surface
```

Maximize that ratio. Everything else is noise.

---

## Product principles (decision filter)

Before any feature or PR:

1. **Does it shrink or protect the public surface?** If it grows surface without layer leverage → no.  
2. **Does it make get/put/commit/range faster or more correct?** If neither → no.  
3. **Can a layer implement it with TX instead?** If yes → layer, not core.  
4. **Does it force multi-node or network?** If yes → wrong repo.  
5. **Would FDB put this in the core?** If no → probably not us either.

---

## Success metrics (focus, not vanity)

| Metric | Target mindset |
|--------|----------------|
| Public types / methods | Fit in one short docs page |
| p99 get / commit (local SSD) | Competitive with serious embeds; tracked in CI benches |
| Correctness | Crash + concurrency via deterministic sim + tests |
| Time-to-layer | A toy secondary index layer in &lt;100 lines using only TX API |
| Dependency / unsafe | `forbid(unsafe)`; minimal deps |

Stars and “supports X” are not success metrics.

---

## Roadmap filtered by focus

Only work that serves the tiny surface + speed + TX bedrock:

| Do now | Why |
|--------|-----|
| WAL, MemTable, SST, get, range | Make the kernel real |
| Local TX (begin/commit, MVCC, OCC) | The superpower |
| Compaction that doesn’t kill write path | Speed under load |
| Simulation + oracle | Trust without surface |

| Defer forever / other product | Why |
|------------------------------|-----|
| Cluster, gRPC, PD | Not this surface |
| SQL, indexes, CDC product | Layers |
| Exotic merge ops, CF zoo | Surface bloat |

---

## Naming the focus for humans

| Bad pitch | Good pitch |
|-----------|------------|
| “RocksDB in Rust with modern research” | “The smallest ACID ordered KV you can build a database on” |
| “Faster fjall” | “FDB-style transactions, in-process, no cluster tax” |
| “Full-featured embed DB” | “Kernel, not kitchen sink” |

---

## Decision

| # | Decision | Status |
|---|----------|--------|
| P1 | Optimize for **power/surface ratio**, not feature count | **Accepted** |
| P2 | Public surface ≈ open + TX get/put/delete/range/commit | **Accepted** |
| P3 | Speed of that path is a first-class goal | **Accepted** |
| P4 | Multi-key ACID is the build-on superpower | **Accepted** |
| P5 | Local library only | **Accepted** (prior) |
| P6 | Research LSM is under the hood, not API surface | **Accepted** |

When in doubt: **delete scope, keep the kernel fast, keep TX sacred.**
