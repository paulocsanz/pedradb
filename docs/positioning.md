# PedraDB positioning: justify use first; polish later

> The embedded KV space is **crowded**. PedraDB only exists if someone has a
> **clear reason to use it**. Deterministic simulation, academic LSM papers,
> and long-term architecture are **how we earn trust later** — not the pitch,
> and not the first milestone.
>
> **Order of proof:** useful kernel → fast enough → correct under stress →
> fancy substrate story. Never the reverse.

---

## 1. Why would anyone use this? (justify first)

Someone reaches for PedraDB instead of the defaults when **all** of this is true:

| Need | Why not the default |
|------|---------------------|
| **Embedded** (in-process library) | Postgres/FDB/TiKV are servers/clusters |
| **Durable ordered KV** | Hash maps / Redis aren’t the source of truth |
| **Multi-key ACID in one commit** | RocksDB/Pebble don’t give real multi-key TX; easy to corrupt “data + index” |
| **Tiny API** | Don’t want RocksDB’s surface or a SQL engine |
| **Rust, no C++ in the hot path** | rust-rocksdb is FFI + RocksDB’s complexity |

**The use case in one line:**

> “I need to update several keys atomically, keep them ordered for scans, survive crash, link a library — and later maybe build a real DB/layer on that.”

Concrete examples (early justification):

1. **App embed** — accounts, jobs, metadata: debit A, credit B, write audit row — **one transaction**.  
2. **Secondary index layer** — `put(row)` + `put(index_entry)` same commit (FDB layer pattern, local).  
3. **State machine under your own protocol** — apply a batch of key updates atomically when *your* code is ready (outer product later; even solo apps need this).

If we cannot demo (1) and (2) **simply and quickly**, papers and sim don’t matter.

### What does *not* justify use (yet)

| Pitch | Problem |
|-------|---------|
| “We have deterministic simulation” | User doesn’t open a crate for your test framework |
| “WiscKey + Monkey + Dostoevsky” | User cares about p99 and not losing data, not paper names |
| “Future multi-node like TiKV” | That’s another product; doesn’t justify *this* library today |
| “More features than fjall” | Crowded field; we lose feature races |

Those become **reasons to trust and stay** after the kernel already justifies itself.

---

## 2. One sentence (product)

**PedraDB is a tiny, fast, pure-Rust library: ordered key-value + multi-key ACID on one machine — so you can build correctly on top without a cluster or a C++ engine.**

Not a server. Not multi-node. Not SQL.

```text
open → begin → get / put / delete / range → commit
```

---

## 3. Order of delivery (justify → then deepen)

| Phase | Ships | Proves |
|-------|--------|--------|
| **A. Justify use** | WAL + memtable + get/put + **TX commit** + basic durability | “I can build a correct multi-key update with a small API” |
| **B. Make it real** | SST, range, crash recovery, compaction that doesn’t fall over | Survives restart; handles more than a toy |
| **C. Make it fast** | Benches, less amp, fewer allocs; *then* research opts if they move numbers | Worth embedding under something serious |
| **D. Make it trusted** | Deterministic sim, fault injection, oracle vs RocksDB where useful | Sleep at night; adopt as substrate |
| **E. Optional later** | Outer multi-node product embeds PedraDB | Scale-out story — **not** required to justify PedraDB |

**Determinism / sim = phase D, not phase A.**  
We may write tests early; we don’t *sell* or *block* the product on a full FDB-style simulator before the API is useful.

**Research LSM = phase C tools**, not the homepage. Ship a correct LSM path first; adopt WiscKey/Monkey/Lazy Leveling when they buy measurable speed/space — still zero extra API surface.

---

## 4. Focus once use is justified

Three constraints, still valid — but **after** the “why use” is real:

### Surface → tiny

Same card API. If a layer can do it with TX → not core.

### Speed → default path fast

Measure get/put/commit. Refuse surface that slows the kernel.

### Build-on → TX is the superpower

Multi-key ACID + order is why this isn’t “just another map on disk.”

---

## 5. Crowded field: how we still justify

| Default choice | When PedraDB wins the “why” |
|----------------|----------------------------|
| **fjall** | You want **TX-first kernel** and a ruthlessly smaller “database kernel” story, not optional TX + broader embed kit |
| **redb** | You need **write-heavy LSM** substrate long-term, not B-tree ACID for lighter embeds |
| **SurrealKV** | You’re **not** building inside Surreal’s product |
| **RocksDB** | You want **Rust + multi-key ACID** without C++ and without bolting TX yourself |
| **FDB/TiKV** | You need **embed / single process**, not a cluster |

Until phase A works, none of that paragraph is earned.

---

## 6. Non-goals (unchanged, keep surface small)

No multi-node, no server, no SQL/indexes in core, no knob zoo, no object-store-first, no Redis types.

---

## 7. Success metrics by phase

| Phase | Metric that matters |
|-------|---------------------|
| **A Justify** | &lt;30 min to a multi-key TX demo; index-layer sketch in tens of lines |
| **B Real** | Crash + reopen keeps committed data; range works |
| **C Fast** | Benches vs a peer on get/put/commit; no silent regressions |
| **D Trust** | Sim/fault tests catch real bugs; oracle where it pays off |

Stars, paper count, and “we planned multi-node” do not justify use.

---

## 8. Decision filter (updated)

1. **Does this make the justify-use path clearer or faster to demo?** If no, defer.  
2. **Does it protect the tiny TX API?**  
3. **Is it speed/correctness of get/put/commit/range?**  
4. **Can a layer do it?** → layer  
5. **Is it sim/papers/cluster storytelling before phase A works?** → later  

---

## 9. Decision log

| # | Decision | Status |
|---|----------|--------|
| P0 | **Justify use first** (useful TX KV kernel) before sim/research as identity | **Accepted** |
| P1 | Power/surface ratio | Accepted |
| P2 | API ≈ open + TX CRUD + range | Accepted |
| P3 | Speed of that path | Accepted |
| P4 | Multi-key ACID = build-on superpower | Accepted |
| P5 | Local library only | Accepted |
| P6 | Sim + papers = later trust/speed, not the pitch | **Accepted** |

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
