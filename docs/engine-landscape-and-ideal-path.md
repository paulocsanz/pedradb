# Storage engine landscape: comparative analysis & the ideal path

> Survey of storage engine architectures and LSM-tree research, with the goal of
> identifying the **asymptotically optimal design path** that nobody has fully
> walked — and why (backward compatibility, testing risk, effort).

All sources are persisted in `docs/references/`.

## Engine comparison matrix

| Engine     | Language | Data structure       | KV-sep | Testing approach                  | Embedded/Distributed | Status         |
|------------|----------|----------------------|--------|-----------------------------------|-----------------------|----------------|
| RocksDB    | C++      | LSM (leveling)       | BlobDB (opt, non-default) | Unit + stress         | Embedded             | Mature, de facto |
| Pebble     | Go       | LSM (leveling)       | No     | Metamorphic + bidirectional compat | Embedded             | Production (CockroachDB) |
| fjall      | Rust     | LSM (leveling)       | Yes (value-log crate) | Unit + fuzz          | Embedded             | Active, growing |
| BadgerDB   | Go       | LSM + value log      | **Native** (WiscKey) | Unit + Jepsen-style nightly | Embedded        | Stable (Dgraph) |
| SlateDB    | Rust     | LSM on object store  | Planned | DST (deterministic simulation test) | Embedded (cloud)  | Early, adopted |
| TiKV       | Rust     | LSM (wraps RocksDB)  | Titan (opt) | Integration + Jepsen       | Distributed (Raft)   | Production (TiDB) |
| FoundationDB | C++ (Flow) | B-tree → Redwood  | N/A    | **Deterministic simulation**      | Distributed          | Production (Apple) |
| WiredTiger | C++      | B-tree or LSM (hybrid) | No   | Unit + integration               | Embedded (MongoDB)   | Mature |
| LMDB       | C        | B+tree (mmap)        | N/A    | Extensive real-world             | Embedded             | Ultra-stable |
| sled       | Rust     | B-link tree (MVCC)   | No     | —                                | Embedded             | **Abandoned** |

---

## Detailed evaluation per engine

### RocksDB — the incumbent, and what it gets wrong
**Source:** [P1] Pebble announcement, [P2] Pebble differences, [D] Dostoevsky, [M] Monkey, [W] WiscKey

The de-facto standard. ~350k LOC of C++. The baseline everyone compares against.
Its fundamental design problems (detailed in `rocksdb-critiques-and-improvements.md`):

- **Write amplification: 50x+** for large datasets (7 levels × 10x size ratio) [W]
- **Read amplification: O(L)** point lookups that check every level [M]
- **Uniform Bloom filter FPR** across all levels (suboptimal allocation) [M]
- **All levels merge equally** even though only the last level's merge matters [D]
- **Frontier of language barrier** (CGO for non-C++ users) [P1]

### Pebble — the best-in-class LSM rewrite
**Source:** [P1], [P2]

Go port that replaced RocksDB in CockroachDB. ~45k LOC. The most successful
clean-room LSM rewrite in production. Key improvements: internal key structs,
batch-as-level, integrated range tombstones, invariant-based pacing, no
artificial write throttling. **Still doesn't adopt** Monkey's Bloom allocation,
Dostoevsky's Lazy Leveling, or WiscKey separation.

### BadgerDB — WiscKey in production
**Source:** [W] WiscKey paper, BadgerDB README

Go LSM with **native key-value separation**. Based directly on the WiscKey
paper (FAST'16). Key insight: only keys go through the LSM tree; values go to
a separate append-only "value log". This **dramatically reduces write
amplification** (2.5x–111x faster than LevelDB for loading, 1.6x–14x faster
for random lookups [W]).

Trade-off: range scans that touch large values require random I/O to the value
log (extra seeks). GC is needed for the value log. Crash consistency is
trickier (relies on append-atomicity of modern filesystems).

**This is the most important architectural choice for PedraDB to consider:**
whether the LSM tree holds full key-value pairs (RocksDB/Pebble/fjall model) or
just keys with values in a separate log (WiscKey/BadgerDB model).

### fjall — Rust LSM, closest peer to PedraDB
**Source:** [F] fjall blog

`#![forbid(unsafe_code)]`, active development (v3.1 as of 2026), ~45k LOC.
Uses standard leveling. Added KV separation (value-log crate) in v2.0.
Custom `Slice` type instead of `Arc<[u8]>`. Per-partition compression.

### SlateDB — cloud-native LSM on object storage
**Source:** SlateDB README

Rust LSM that writes SSTs to S3/GCS/ABS instead of local disk. "Bottomless
storage." Uses `object_store` crate for pluggable backends. Batches writes to
amortize PUT costs. Has DST (deterministic simulation test) framework.
Different niche (latency-tolerant, high-durability) but validates Rust + LSM.

### FoundationDB — the testing pioneer
**Source:** FDB architecture + testing docs

Not an LSM tree — uses B-tree (SQLite-derived "ssd" engine), moving to Redwood
(a custom B+tree with prefix compression). Its contribution is **deterministic
simulation testing**: running an entire cluster (network, disk, machines) in a
single-threaded process, with perfect repeatability. ~1 trillion CPU-hours of
simulation. Finds bugs that no integration test ever would.

The architecture is also notable: **decoupled roles** (commit proxies, resolvers,
transaction logs, storage servers) that scale independently. Reads scale
linearly (clients go directly to sharded storage servers).

### TiKV — distributed KV on top of RocksDB
**Source:** General knowledge (tikv.org)

Rust distributed KV store using Raft for consensus. **Wraps RocksDB** as its
local storage engine — doesn't reimplement the LSM. Adds an MVCC layer on top.
Has Titan (optional KV separation, inspired by WiscKey). Production in TiDB.

### WiredTiger — B-tree/LSM hybrid
**Source:** General knowledge

MongoDB's storage engine since 3.2. Offers both B-tree and LSM modes. In
practice, B-tree is dominant. Notable for its **cache-aware** design and
compression. Shows that B-trees remain competitive for read-heavy workloads.

### LMDB — the mmap B+tree
**Source:** General knowledge

Memory-mapped B+tree. Single-writer, multi-reader with MVCC via COW.
Unbelievably fast for read-heavy workloads. Zero-maintenance. But doesn't scale
for writes, single-writer bottleneck, fixed-size mmap. A different philosophy:
"do less, do it perfectly."

### sled — cautionary tale
**Source:** General knowledge

Rust B-link tree with MVCC. Promised a lot, **abandoned/maintainer gone silent**.
Important lesson for the Rust ecosystem: a storage engine needs sustained
engineering over years, not just a clever architecture.

**PedraDB response (documented):** do not copy sled storage; optional
**sled-shaped API layer** on LSM; preserve format/iterator options so
WiscKey/Monkey/Lazy Leveling stay possible — see
[`performance-ceiling-option-preservation-and-sled-layer.md`](performance-ceiling-option-preservation-and-sled-layer.md).

---

## The asymptotically ideal path (and why nobody walked it)

### What research says is optimal

Three academic results define the Pareto-optimal LSM design:

| Optimization | Paper | Effect | Adopted by |
|-------------|-------|--------|------------|
| **KV separation** | WiscKey [W] FAST'16 | Write amp: O(T^L) → O(T·L). Values written once, never compacted. | BadgerDB (native), fjall (opt), RocksDB BlobDB (opt, non-default) |
| **Optimal Bloom allocation** | Monkey [M] SIGMOD'17 | Lookup: O(L·e^(-M/N)) → O(e^(-M/N)). Constant regardless of dataset size. | **Nobody in production** |
| **Lazy Leveling** | Dostoevsky [D] SIGMOD'18 | Write amp reduced ~10x vs leveling, same lookup/space bounds. Tiering in small levels, leveling only in largest. | **Nobody in production** |

Combined, these three optimizations would produce an LSM that:
- Writes with **~5-10x less amplification** than RocksDB
- Does point lookups in **O(1) amortized I/O** regardless of dataset size
- Uses **less space** (less stale data from over-merging)
- Handles **large values** efficiently (no write amp from compacting them)

### Why nobody has combined them

1. **Backward compatibility.** RocksDB and Pebble have committed on-disk formats.
   Changing the SST format, Bloom filter allocation, or compaction strategy breaks
   every existing deployment. RocksDB added BlobDB as an opt-in layer, not a default.
   Pebble explicitly maintains bidirectional format compatibility with RocksDB.

2. **Testing risk.** A fundamentally different compaction strategy (Lazy Leveling)
   requires exhaustive testing across failure modes (crash, corruption, partial
   writes, concurrent compactions). Without FoundationDB-level simulation testing,
   you can't be confident it's correct. **Only FDB has invested in deterministic
   simulation** — and they use a B-tree, not an LSM.

3. **Cold-start effort.** Combining all three requires a clean-room rewrite (not a
   fork). Pebble did a clean-room rewrite but chose compatibility with RocksDB's
   format over adopting academic optimizations. The academic prototypes (Dostoevsky,
   Monkey) were built **on top of** RocksDB/LevelDB — they proved the math but
   didn't ship production-quality implementations.

4. **The expertise barrier.** Understanding why Lazy Leveling works requires
   reading the Dostoevsky cost model. Understanding why Bloom allocation matters
   requires reading Monkey's geometric series argument. Most engineers building
   storage engines are practitioners, not academics. The gap between the research
   frontier and production practice is ~5-10 years.

### The gap — where PedraDB can land

```
    Academia                          Production practice
    (proven, not shipped)             (shipped, suboptimal)

    ┌──────────────┐                  ┌──────────────┐
    │ Monkey Bloom │                  │ Uniform Bloom│
    │ allocation   │  ←── 5-10 yr ──→ │ (RocksDB)    │
    ├──────────────┤    gap           ├──────────────┤
    │ Lazy Leveling│                  │ Pure leveling│
    │ (Dostoevsky) │  ←── never ────→ │ (RocksDB/    │
    ├──────────────┤    crossed       │  Pebble)     │
    │ WiscKey KV   │                  │ LSM only     │
    │ separation   │  ←── partially → │ (+opt BlobDB)│
    └──────────────┘                  └──────────────┘
              ↓                               ↓
         ┌─────────────────────────────────────────┐
         │           THE PEDRADB OPPORTUNITY         │
         │  Clean-room Rust: adopt ALL THREE +       │
         │  deterministic simulation testing (FDB)   │
         └─────────────────────────────────────────┘
```

**PedraDB's thesis:** a clean-room Rust LSM that combines WiscKey separation +
Monkey Bloom allocation + Dostoevsky Lazy Leveling + FoundationDB-style
simulation testing is **asymptotically better** than any production LSM today,
and the only reason it doesn't exist yet is the backward-compatibility / testing-
risk / effort barrier — which a new project, with no users yet, doesn't face.

---

## Decision framework: what PedraDB should adopt

| Feature | Adopt? | Rationale | Priority |
|---------|--------|-----------|----------|
| LSM-tree core | ✅ | Best for write-heavy workloads, proven design | P0 |
| KV separation (WiscKey) | ✅ | Biggest write-amp win; design it in from day 1 | P1 |
| Monkey Bloom allocation | ✅ | Zero runtime cost, pure design decision | P1 |
| Lazy Leveling / Fluid LSM | ✅ | Biggest compaction improvement; needs cost model | P2 |
| Deterministic simulation | ✅ | The only way to trust a novel compaction strategy | P0 |
| `forbid(unsafe_code)` | ✅ | Rust safety, already adopted | P0 |
| Rust-native types (InternalKey, Slice) | ✅ | Pebble lesson: avoid encoded keys | P0 |
| Object storage backend (SlateDB model) | ❌ (for now) | Different niche; local-disk first | Future |
| Distributed consensus (Raft, TiKV model) | ❌ (for now) | Layer on top later, not in the engine | Future |

---

## Sources

| Ref | Source | File |
|-----|--------|------|
| [W] | Lu et al., "WiscKey: Separating Keys from Values in SSD-conscious Storage", FAST 2016 | `wisckey-fast2016.pdf` |
| [M] | Dayan & Idreos, "Monkey: Optimal Navigable Key-Value Store", SIGMOD 2017 | `monkey-sigmod2017.pdf` |
| [D] | Dayan & Idreos, "Dostoevsky", SIGMOD 2018 | `dostoevsky-sigmod2018.pdf` |
| [P1] | "Introducing Pebble", Cockroach Labs, 2020 | `pebble-announcement-2020.md` |
| [P2] | Pebble vs RocksDB differences doc | `pebble-vs-rocksdb-differences.md` |
| [F] | fjall 2.0 announcement | `fjall-2-announcement.html` |
| [FDB] | FoundationDB architecture + testing docs | fetched live |
| [S] | SlateDB README | fetched live |
| [B] | BadgerDB README | `badger-github.md` |
