# Performance ceiling, option preservation, and sled-shaped API layer

**Status:** product doctrine + engineering plan (living)  
**Updated:** 2026-08-12  
**Audience:** anyone changing core format, iterators, TX, compaction, or public API  

**Complements:**

| Doc | Role |
|-----|------|
| [`plan-limitations-and-failure-modes.md`](plan-limitations-and-failure-modes.md) | Where the grail can fail later |
| [`engine-landscape-and-ideal-path.md`](engine-landscape-and-ideal-path.md) | LSM research path (WiscKey / Monkey / Lazy Leveling) |
| [`rocksdb-critiques-and-improvements.md`](rocksdb-critiques-and-improvements.md) | Pebble/Rocks lessons → concrete design rules |
| [`competitive-landscape-rust.md`](competitive-landscape-rust.md) | sled as cautionary tale among Rust peers |
| [`doctrine-primitives-and-api-layers.md`](doctrine-primitives-and-api-layers.md) | Kernel tiny; products = layers |
| [`rfc/0014-rocks-pebble-redwood-maturity.md`](rfc/0014-rocks-pebble-redwood-maturity.md) | Near-term maturity slices |
| [`rfc/0012-research-decisions.md`](rfc/0012-research-decisions.md) | Measured non-ship of vlog / Lazy Leveling (for now) |
| [`positioning.md`](positioning.md) | Justify use first; avoid “next sled” |

---

## 1. Why this document exists

After micro-optimizations (allocs, caches, SIMD), remaining performance is the
**architectural ceiling**. Teams often discover too late that an important
optimization (value log, multi-level compaction, streaming range, multi-writer)
is **impossible** without a rewrite — the sled failure mode: ambitious design,
perpetual alpha, format breaks, never a boring stable kernel.

This doc records:

1. What actually sets the **performance ceiling** (vs peers).  
2. **Option-preservation** rules so we do not paint into a corner.  
3. Honest **B-tree vs LSM** and “B-tree-class reads without a second store.”  
4. A plan for a **sled-shaped API layer** on PedraDB (compat / DX), not in core.  
5. A **phased performance plan** toward Rocks/Pebble-class utility and better amp.

**Doctrine reminder:** PedraDB core stays:

```text
open → begin → get / put / delete / range → commit
```

Rich APIs (BTreeMap/sled, etcd, SQL) are **layers**.

---

## 2. What sets the ceiling (not micro-benches)

| Layer | What it limits | Change later? |
|-------|----------------|---------------|
| **Structure family** (LSM vs B-tree vs sled pagecache-log) | Write/read/space amp; concurrency model | **Almost never** — new engine |
| **On-disk format** (WAL, SST, MANIFEST, keys) | Lazy blocks, compression, filters, vlog, streaming | **Yes if versioned**; no if opaque monolith |
| **Internal API** (`InternalKey` struct, merging iterators, `Env`) | Zero-copy, batch-as-level, group commit, DST | **Yes if seams clean** |
| **Public TX / durability contract** | Multi-core write QPS; commit latency floor | Hard if promised early |
| **Correctness + ops maturity** | Trust under compaction/crash; field p99 | Always work — no sim → social ceiling |

Micro-opts move **constants**. Architecture moves **asymptotics** (levels, write amp, I/Os per get).

### Physics we accept (not “corners”)

| Limit | Why |
|-------|-----|
| Durable commit ≈ `fdatasync` (or Raft quorum) | Disk / network; group commit only amortizes under concurrency |
| Hot key = one writer | No structure scales celebrity keys without app/region split |
| Read-heavy + cold cache + many levels | LSM can lose to B-tree/mmap embeds — **family choice** |
| Multi-region TX latency | RTTs; not an SST problem |
| “Faster than Redis and always durable” | Wrong category |

A **corner** is when a classic LSM optimization becomes **impossible** because of format/API.  
A **family choice** is when redb/LMDB would win a read-heavy embed and we still chose LSM for substrate/write path.

---

## 3. Ceilings vs peers (honest)

| System | Family | Design ceiling (when mature) | Where it does **not** win | Lesson for PedraDB |
|--------|--------|------------------------------|---------------------------|--------------------|
| **RocksDB** | LSM + knobs | High write ingest, ecosystem, lore | High write amp; read amp; huge surface | Incumbent ≠ optimal amp; do not copy knob zoo |
| **Pebble** | Clean LSM (Rocks-compat) | Fewer footguns; batch-as-level; native keys | Still not Monkey/LL/WiscKey default | Clean-room wins maintainability; **compat freia** paper path |
| **Redwood / FDB** | B-tree | Reads/scans; world-class sim; product TX | Not Rocks-class local write substrate | B-tree still production-grade; **sim is the superpower** |
| **redb / LMDB** | B-tree COW/mmap | Read-heavy embed; simplicity | Massive concurrent ingest | If that is the product, **use them** — do not pretend LSM always wins |
| **sled** | B-link + pagecache + log | In theory: log writes + tree reads + lock-free | Space amp, GC, unfinished | Novel unfinished design → social + real ceiling; **do not copy** |
| **Badger / Titan** | LSM + value log | Large values without compacting payload | Large-value ranges → random I/O; vlog GC | WiscKey changes **large-value write amp** ceiling |
| **fjall** | Safe Rust LSM | Shipable embed; optional TX/KV-sep | Not TX-first substrate story | Closest peer; do not become “fjall with more options” |
| **PedraDB (target)** | Clean-room LSM + multi-key ACID | Path to better amp **if** options preserved | Early; single-writer; simple compact today | Potential > Rocks amp only with levels + filters + optional vlog + discipline |

### vs sled specifically

| Claim | Reality |
|-------|---------|
| “LSM write + B+tree read” | Marketing; GC/space often worse; never proven at Rocks maturity |
| Lock-free = always faster | Helps multi-core contention; loses many single-thread / low-contention cases |
| API is the product | **API is portable**; we can offer sled-shaped DX on LSM storage |
| Pure Rust = production default | Still `unsafe`; long alpha; README sends serious users to SQLite/Rocks/LMDB |

**We do not aim to beat sled’s unfinished theoretical storage.**  
We aim to beat **sled’s product failure**: ship a small kernel, stable format story, and optional DX layer.

### Performance goal (when mature)

| Axis | Goal |
|------|------|
| **Utility / ops** | Pebble-class: checkpoint, stats, verify, clear durability, sane compact |
| **Write amp (large values)** | Path to **better than classic Rocks** via optional WiscKey when measured |
| **Point lookup amp** | Bloom + bounds + (later) Monkey-style allocation; not full-table walk |
| **Range** | Streaming merge; not materialize whole DB |
| **Read-heavy small embed** | **Honest:** redb/LMDB may win — document; do not lie |
| **Local multi-key ACID** | **Win:** core contract Rocks lacks |
| **sled DX** | Layer parity for common ops; not lock-free pagecache |

---

## 4. B-tree vs LSM (and “B-tree reads without extra storage”)

### 4.1 Three different things

| Thing | Meaning |
|-------|---------|
| `std::BTreeMap` | In-memory only |
| **Disk B-tree** (redb, LMDB, Redwood) | Update in-place / COW pages on disk |
| **LSM** (Rocks, Pebble, fjall, Pedra) | Append + SST + compaction |

### 4.2 Tradeoff

| | Disk B-tree | LSM |
|--|-------------|-----|
| Writes | Page updates / COW | Append hot path; better **ingest** |
| Point read | Excellent when hot/mmap | Good with Bloom + cache; multi-level cost |
| Range | Excellent leaf walk | Merge of mem + SSTs |
| Space | Stable if maintained | Tombstones until compact |
| Ops | Often “zero maintenance” (LMDB) | Compaction storms possible |
| Substrate for write-heavy / TiKV-class | Weaker fit | Strong fit |

**PedraDB chose LSM** to sit in the Rocks substrate family, not LMDB/redb family.
That is intentional. It is also a **ceiling choice** for pure read-heavy embeds.

### 4.3 “B-tree-class reads” **without** a second on-disk B-tree

We will **not** dual-write to an LSM + a disk B-tree for the same keys (double
space, double crash story, sled-level complexity).

Instead, approximate B-tree **read shape** with LSM engineering:

| B-tree read property | LSM technique (no extra primary store) |
|----------------------|----------------------------------------|
| Few I/Os per point get | Bloom + key bounds + block index; skip irrelevant files |
| Hot data in RAM | Table cache + block cache (P1+); optional pin / warm |
| Ordered scan without huge merge | Leveled layout (fewer overlapping files); streaming `MergingIterator` |
| Stable leaf locality | Compression + block size tuning; prefix extraction in blocks |
| Predictable latency | Compaction pacing; avoid whole-DB merge as steady state |
| “Map-like” API | **Layer only** — see §7 |

**In-memory** we may keep a `BTreeMap` (or skiplist) for the **memtable / TX staging** —
that is not a second durable store; it is the standard LSM write buffer.

Optional later (not dual primary store):

- **Secondary index** as keys in the **same** LSM (TX maintains both) — FDB layer pattern.  
- **Read replicas / RO snapshot** for scale-out reads — product layer, same format.  
- **Hybrid engines** (WiredTiger-style) only if a measured product needs them — out of v1 scope.

**Rejected:** sled-style lock-free pagecache + log as default storage (rewrite trap, space amp).

---

## 5. Option preservation (anti-corner checklist)

Every core PR/RFC should pass this filter. “No” = ceiling debt.

### 5.1 Format & data model

| # | Rule |
|---|------|
| F1 | **Version + magic** on WAL, SST, MANIFEST, checkpoint meta; unknown version = fail-closed |
| F2 | Readers accept **N prior** versions; writers bump version deliberately with tests |
| F3 | SST is **blocks + sparse index + filter + properties** — not only a flat entry blob forever |
| F4 | `InternalKey { user_key, seq, kind }` as **struct** in internal APIs (Pebble lesson); encode only at I/O edge |
| F5 | `kind` extensible: Put, Delete, **room for** RangeDelete / Merge |
| F6 | Values modeled so **inline vs remote** is possible: e.g. mental/type path to `ValueRef::Inline | VLog(addr)` even while only Inline ships |
| F7 | MANIFEST metadata has room for **level, size, smallest/largest, seq bounds** even if compact is still simple |
| F8 | Never ship “format will break until 1.0 with manual export only” (sled warning) without a migration story |

### 5.2 Iterators & memory

| # | Rule |
|---|------|
| I1 | **One** merging read path (memtable + levels + optional batch) |
| I2 | Public APIs must not **require** materializing entire ranges (`Vec` of whole DB) |
| I3 | Lazy **block** load path exists or is planned before huge SSTs in production embeds |
| I4 | Memtable behind interface (`insert` / `get` / `range`) — BTreeMap today, skiplist/arena later without `Db` rewrite |
| I5 | Batch treatable as **another LSM level** (Pebble) when multi-op reads need it |

### 5.3 Commit, durability, concurrency

| # | Rule |
|---|------|
| C1 | Durability is a **policy** (`Sync` / group / relaxed), not a hardcoded fsync deep in `put` with no knobs |
| C2 | Document **single-writer TX** honestly; multi-writer/OCC is evolution, not a silent promise |
| C3 | Scale-out write path is **multiple Db / regions above** Pedra, not multi-process same directory |
| C4 | Group commit design must not assume “only one thread ever commits” if OCC is on the roadmap |

### 5.4 Compaction & amp

| # | Rule |
|---|------|
| A1 | Compaction **policy** pluggable (whole-merge → leveled → lazy leveling) without rewriting SST readers |
| A2 | Emit **amp metrics** (bytes written / bytes ingested, levels, compaction time) early |
| A3 | Version GC / tombstone drop piggybacks merge path, not a one-off script |
| A4 | Research freeze (do not ship LL/vlog yet) ≠ code that **makes them impossible** |

### 5.5 What not to do early

| # | Rule |
|---|------|
| X1 | No sled pagecache-log rewrite “for theoretical amp” |
| X2 | No Rocks knob zoo in core |
| X3 | No watch/merge/triggers/SQL in `pedradb-core` |
| X4 | No dual on-disk B-tree + LSM for the same primary data |
| X5 | No README claims of Monkey+LL+WiscKey before merging iterators + levels work |
| X6 | No mass `unsafe` for micro-wins without SAFETY audit budget |

### 5.6 Current PedraDB status (preserve these)

| Already good | Why it preserves ceiling |
|--------------|--------------------------|
| LSM + `InternalKey` struct | Rocks family; Pebble internal API lesson |
| SST versioned (v1→v4), bloom, bounds | Lazy load / filters / compression path |
| `Env` / Host / fault seams | DST, io_uring, alternate backends |
| Multi-key TX in core | Layers do not reimplement consistency |
| Tiny public surface | Avoids sled feature surface |
| Bloom shipped for **shape** (not only bench) | Read amp path |
| WiscKey / Lazy Leveling **not shipped** but not forbidden by family choice | Room to open with metrics |

### 5.7 Active corner risks (close these windows)

| Risk | Why | Close with |
|------|-----|------------|
| Range materializes large `Vec`s | OOM; bad public dependency | Streaming range (RFC-0014 P1.1) |
| Full SST entry vectors in RAM | RSS; weak cache | Lazy block load (P1.2) |
| Whole-merge only forever | Write amp / latency | Multi-level MANIFEST + policy (P1.3) |
| Values always conceptually “inline forever” | Large-value amp | `ValueRef` / threshold hook before TB-scale data |
| Single-writer assumed by all layers | Local multi-core write QPS | Document; ConcurrentDb / OCC path |
| Sync default vs async peer benches | False “we are slow” | Policy + apples-to-apples benches |

---

## 6. Learning from innovators (steal the lesson, not the trap)

| Source | Steal | Do not copy |
|--------|-------|-------------|
| **Pebble** | Struct keys, batch-as-level, less surface, range deletes done right | Rocks format handcuffs if we want paper defaults |
| **RocksDB** | Bloom, caches, checkpoint, properties, compaction lore | 350k LOC, infinite options |
| **Dostoevsky / Monkey / WiscKey** | Cost models; what uniform leveling/filters get wrong | Ship all three on day one without iterator stack |
| **FDB / Redwood** | Deterministic simulation, fail injection | Switch core to B-tree unless product demands it |
| **Badger / Titan** | Value log ops (GC feedback) | Vlog without value-size metrics |
| **fjall** | Safe Rust ship discipline | Optional-everything kit race |
| **sled** | Map-like DX; seriousness about hazards | Eternal rewrite, unstable format, novel storage unfinished |
| **Scylla / Seastar** | Shared-nothing scaling ideas at product layer | Couple runtime to storage engine |

**Method:** get numbers before theories; structural changes need RFC + format version + reopen criteria; compare durability apples-to-apples.

---

## 7. Sled-shaped API layer (`pedra-map` / sled-compat)

### 7.1 Goals

| Goal | Non-goal |
|------|----------|
| DX close to sled / `BTreeMap<[u8],[u8]>` for embeds | Reimplement sled storage |
| Sit **on** PedraDB TX + ordered KV | Grow `pedradb-core` surface |
| Trees via **key prefixes** (v1) | Physical multi-LSM keyspaces in core v1 |
| CAS / batch / multi-key TX via core TX | Optimistic lock-free pagecache |
| Optional watch **in-process** | Distributed watch (that is DCS/stream product) |
| Document durability mapping | Promise sled’s default 500ms flush semantics silently |

### 7.2 Placement

```text
┌─────────────────────────────────────────────────────────────┐
│  App code                                                   │
├─────────────────────────────────────────────────────────────┤
│  pedra-map / sled-compat  (LAYER — new crate, not core)     │
│  open, Tree, insert, get, remove, range, cas, transaction   │
│  open_tree(prefix), apply_batch, flush, optional watch      │
├─────────────────────────────────────────────────────────────┤
│  pedradb-core                                               │
│  open · begin · get/put/delete/range · commit · flush       │
│  durability policy · SST/WAL/MANIFEST                       │
└─────────────────────────────────────────────────────────────┘
```

Same doctrine as DCS/SQL: **encode on keys + TX**.

### 7.3 API mapping

| Sled-like API | PedraDB mapping |
|---------------|-----------------|
| `open(path)` | `Db::open` / `ConcurrentDb::open` behind `Arc`/lock as needed |
| `insert(k,v)` → old | TX or single-key put; optional get-for-old in same TX |
| `get` / `remove` | get / delete |
| `range(a..b)` | streaming range when core has it; limited range until then |
| `compare_and_swap` | `begin` → get → match → put/delete → commit |
| `apply_batch` | one TX multi put/delete |
| `transaction(\|t\| …)` | map to core TX; **document** single-writer vs sled OCC retries |
| `open_tree(name)` | prefix `name || 0x00 || user_key` (or length-prefixed) |
| `flush` / `flush_async` | core durability / WAL sync policy |
| `generate_id` | counter key + TX, or layer-local generator |
| `watch_prefix` | **layer**: in-process pubsub after successful commit (best-effort); not durability primitive |
| `IVec` | `bytes::Bytes` or thin wrapper (clone-cheap) |
| Merge operators / triggers | **out of v1 layer**; app or later trigger layer |

### 7.4 Semantic differences (document in crate README)

| Topic | Sled lore | Pedra layer |
|-------|-----------|-------------|
| TX model | Optimistic retries in closure | Start: single-writer / mutex; later OCC if core supports |
| Durability | Background fsync ~500ms default | **Explicit** policy; default follows core (often fsync on commit) — **faster to document than to silently match sled** |
| Multi-process | Not supported | Same: one process per data dir |
| Format | Broke toward 1.0 | Core versioned format; layer adds no second format |
| Multi-tree isolation | Separate trees | Prefix isolation only unless keyspaces land later |
| Performance | Lock-free story | LSM + caches; **same storage as raw Pedra** |

### 7.5 Illustrative surface (not normative code)

```rust
// crates/pedra-map — future; API sketch only
pub struct Db { /* wraps pedradb_core::… */ }
pub struct Tree { /* prefix + Db handle */ }

impl Tree {
    pub fn insert(&self, key: &[u8], value: &[u8]) -> Result<Option<IVec>>;
    pub fn get(&self, key: &[u8]) -> Result<Option<IVec>>;
    pub fn remove(&self, key: &[u8]) -> Result<Option<IVec>>;
    pub fn compare_and_swap(
        &self,
        key: &[u8],
        old: Option<&[u8]>,
        new: Option<&[u8]>,
    ) -> Result<CasResult>;
    pub fn range(&self, bounds: impl RangeBounds<Vec<u8>>) -> impl Iterator<…>;
    pub fn transaction<F, T>(&self, f: F) -> Result<T>
    where
        F: Fn(&mut TxalTree) -> Result<T>;
}

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Self>;
    pub fn open_tree(&self, name: &str) -> Result<Tree>;
    pub fn flush(&self) -> Result<()>;
}
```

### 7.6 Delivery slices (layer)

| Band | Deliverable | Depends on core |
|------|-------------|-----------------|
| **L0** | Crate `pedra-map`: open, get, insert, remove, flush over ConcurrentDb/mutex | Core put/get/TX |
| **L1** | `open_tree` prefixes + `compare_and_swap` + `apply_batch` + `transaction` | Multi-key TX |
| **L2** | Streaming `range` adapter; `IVec`/`Bytes` ergonomics | RFC-0014 streaming range |
| **L3** | Optional in-process `watch_prefix` (lossy under lag; document) | Commit hooks or post-commit notify |
| **L4** | Optional `sled` API names behind feature; migration notes from sled 0.34 | Stability of L1 |
| **Out** | Merge ops, multi-process, lock-free storage, disk format of sled | — |

**Acceptance:** secondary-index example rewritable with `Tree` + `transaction` in tens of lines; no new on-disk format; benches of layer overhead &lt; small constant vs raw core for get/put.

### 7.7 Why this does not fight performance

The layer is **O(1) prefix concat + TX calls**. Ceiling remains the **kernel**.  
Optimizing “sled performance” means optimizing Pedra LSM (below), not inventing a second engine.

---

## 8. Performance plan (kernel) — toward peer-class ceiling

Phased so we never block higher amp on missing primitives.

### Phase K0 — Correct shape (mostly done / RFC-0014 P0)

- WAL + TX + durability contract documented  
- SST blocks + bloom + key bounds  
- Checkpoint / stats / verify  
- Fail-closed CRC; Env seams  

**Gate:** no silent wrong; crash tests green.

### Phase K1 — Memory & read path (RFC-0014 P1)

| Item | Ceiling effect |
|------|----------------|
| Streaming range | Range without OOM; B-tree-like scan API |
| Lazy block load | Point get I/O + RSS |
| Table/block cache | Hot read ≈ memory map latency class |
| Multi-level or size-tiered compact | Write amp + read amp (fewer overlaps) |
| Optional compression | Space + I/O |

**Gate:** `range` / large SST benches; RSS under bound; amp metrics exported.

### Phase K2 — Write path & concurrency

| Item | Ceiling effect |
|------|----------------|
| Group commit / durability policy clarity | Commit latency under load |
| Memtable interface → skiplist/arena if CPU-bound | Write CPU |
| Multi-writer OCC or documented sharding | Multi-core write QPS |
| Compaction pacing | p99 under sustained write |

**Gate:** baseline benches vs fjall/Rocks **same sync policy**; no death spirals on large batches (fixed memtable pressure).

### Phase K3 — Research amp (measured reopen)

| Item | Paper | When to open |
|------|-------|--------------|
| Monkey-style bloom budget | Monkey | Many levels + memory budget matters |
| Lazy Leveling / fluid | Dostoevsky | Levels exist; amp metrics show leveling tax |
| Value log | WiscKey | Value size histogram is large-value heavy |

**Gate:** RFC + format version + sim/fault tests; never README-first.

### Phase K4 — Host I/O

| Item | Ceiling effect |
|------|----------------|
| io_uring / async Env (Linux) | Throughput under deep queue |
| Better allocate / copy reduction | CPU on get/put |

**Gate:** only after K1 iterators/caches exist (otherwise wrong bottleneck).

### Explicit non-goals for ceiling chasing

- Dual B-tree primary store  
- Sled pagecache rewrite  
- Matching LMDB mmap latency on cold multi-level LSM without cache  
- Redis-class durable p99 on poor cloud disks without group commit + hardware honesty  

---

## 9. “As fast as X” decision matrix

| If user means… | Answer | Pedra action |
|----------------|--------|--------------|
| As fast as **sled insert in cache** | Layer + core hot path; not sled GC | K1 caches; avoid vlog until needed |
| As fast as **RocksDB async put** | Match **async** durability in bench | Publish dual benches (sync vs async) |
| As fast as **RocksDB sync put** | fsync-bound; group commit | K2 policy |
| As fast as **Pebble under CRDB** | Years of pacing + levels | K1–K2; ops maturity |
| As fast as **redb read-heavy** | May lose | Document; or recommend redb for that niche |
| As fast as **LMDB mmap read** | May lose cold/multi-level | Block cache; do not dual-engine |
| As **ergonomic as sled** | Layer | §7 L0–L2 |

---

## 10. Engineering gates (process)

1. **Numbers before theory** — attach `cargo bench -p pedradb-core --bench baseline` (and future peers) to research reopens.  
2. **Format change** → version bump + read-old test + short migration note.  
3. **New public API in core** → must pass doctrine (else layer).  
4. **Structural opt** → update this doc’s “active corner risks” and RFC-0014/0012 status.  
5. **Layer sled-compat** → no second durability story; link to core `WriteOptions`.  
6. **Claim “production substrate”** → only after K1 + fault hunts + backup/checkpoint story (see robustness doc).

---

## 11. Work items (tracker)

| ID | Item | Band | Status |
|----|------|------|--------|
| DOC-1 | This document | — | **done** |
| K1.1 | Streaming range | P1 | todo (RFC-0014) |
| K1.2 | Lazy block load | P1 | todo |
| K1.3 | Multi-level compact + MANIFEST levels | P1 | todo |
| K1.4 | Block/table cache | P1 | partial / todo |
| K2.1 | Durability policy matrix + group commit design | P1–P2 | todo |
| K2.2 | Memtable trait / skiplist if measured | P2 | todo |
| K2.3 | Multi-writer OCC or shard docs | P2 | todo |
| K3.1 | ValueRef type + threshold (even if vlog off) | P1 design | todo |
| K3.2 | WiscKey reopen | P2 measured | non-ship until evidence |
| K3.3 | Lazy Leveling reopen | P2 measured | non-ship until evidence |
| K3.4 | Monkey bloom budget | P2 | after multi-level |
| L0 | `pedra-map` crate L0 | layer | **todo** |
| L1 | CAS / batch / transaction / open_tree | layer | todo |
| L2 | Streaming range adapter | layer | after K1.1 |
| L3 | Optional watch_prefix | layer | todo |
| M1 | Amp + RSS metrics in `Db::stats` | P1 | extend stats |

---

## 12. Summary

| Question | Answer |
|----------|--------|
| Are we afraid of the right thing? | **Yes** — architectural ceiling and format corners |
| Are we already in a sled corner? | **No** — LSM + versioned SST + Env + tiny TX API |
| B-tree better than LSM? | **Workload-dependent**; we chose LSM; no dual store |
| B-tree reads without extra storage? | **Bloom + cache + levels + streaming merge**, not a second B-tree |
| Sled API? | **`pedra-map` layer**; same Pedra storage; prefixes for trees |
| As fast as Rocks/Pebble/sled? | **Utility → Pebble path; amp → measured research; sled DX → layer; sled storage → no** |

**One line:** preserve options in format and iterators, ship a boring fast LSM kernel, put sled ergonomics in a layer, and chase paper-level amp only after the merge stack and metrics exist.
