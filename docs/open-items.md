# Open items: PedraDB engineering status

> Living document. Updated whenever a design decision is resolved, a research
> item is closed, or a new open question emerges. The authoritative source for
> "what's done, what's next, what's unresolved."

Last updated: 2026-08-10

---

## Current state at a glance

```
 pedradb-core    WAL ✅  | MemTable ⏳  | TX 🔲  | SST 🔲  | Compaction 🔲
 pedradb-sim     placeholder
 pedradb-oracle  trait defined, RocksDB bindings behind feature flag
 pedradb-cli     version + wal commands

 Tests: 11 passing (WAL)     Clippy: 0 warnings     unsafe: #![forbid]
```

---

## 1. Roadmap status

| # | Slice | Status | Key deliverables | Blocked by |
|---|-------|--------|-----------------|------------|
| 0 | WAL | ✅ done | Block format, masked CRC32C, fragmentation, recovery | — |
| 1 | InternalKey + MemTable | ⏳ next | `InternalKey` struct, sorted in-memory map, seqnum, `Slice` type, fixed arena | — |
| 2 | Transaction manager | 🔲 | MVCC snapshot isolation, OCC conflict detection, commit pipeline | Slice 1 |
| 3 | Transactional API | 🔲 | `Transaction { get, put, delete, range_read, commit, abort }` | Slice 2 |
| 4 | SST + flush | 🔲 | Block-based SST, Monkey Bloom, WiscKey value log, flushable batches | Slices 1–2 |
| 5 | Get + range scan | 🔲 | Merged iterator (MemTable ∪ SSTs), MVCC filter, range tombstones | Slice 4 |
| 6 | Compaction | 🔲 | Lazy Leveling, invariant-based pacing, cost model | Slice 4 |
| 7 | Version GC | 🔲 | MVCC version reclaim, value-log GC, backpressure | Slices 4–6 |
| 8 | Deterministic simulation | 🔲 | Disk/time/crash modeling, reproducible runs | Slices 2–6 |
| 9 | Cross-validation harness | 🔲 | Oracle diff vs RocksDB | Slices 4–6 |

**Next action:** Slice 1 (InternalKey + MemTable).

---

## 2. Open design decisions (unresolved)

These are questions where the general direction is known but the specific
approach hasn't been finalized. Each needs a concrete decision before its target
slice can be implemented.

### 2.1 Version GC strategy (Slice 7)

**Question:** How does PedraDB reclaim old MVCC versions?

**Options:**
- **(a) Stop-the-world pause:** scan all versions, remove those older than the
  oldest active snapshot. Simple but causes latency spikes.
- **(b) Incremental GC:** reclaim versions during compaction (piggyback). No
  pause but versions live longer.
- **(c) "Snapshot too old" error:** like FDB — if a snapshot is too old, abort
  the transaction. Safety valve, not primary mechanism.

**Likely answer:** (b) as primary + (c) as safety valve. Needs benchmarking.

### 2.2 Value-log GC strategy (Slice 7)

**Question:** How does PedraDB garbage-collect the WiscKey value log?

**Context:** Values are written append-only. When a key is overwritten or
deleted, the old value becomes garbage. The value log needs periodic
compaction to reclaim space.

**Options:**
- **(a) Online GC:** background thread scans the log, discards orphaned values,
  rewrites live ones. Like BadgerDB's approach.
- **(b) Batch GC:** piggyback on LSM compaction — when a key is compacted,
  discard its old value-log entry.
- **(c) Generational:** split value log into segments, GC the oldest first
  (like generational GC in language runtimes).

**Likely answer:** (a) + (c). BadgerDB and fjall both use online + segment-based.
Needs study of their implementations.

### 2.3 Backpressure strategy (Slice 7)

**Question:** When write rate exceeds flush/compaction capacity, what does
PedraDB do?

**Context:** Pebble's lesson [P2] is: no artificial delays (they increase
latency without benefit in open-loop). But something must bound the system.

**Options:**
- **(a) Explicit stall:** block new writes until L0 is drained. Honest but harsh.
- **(b) Adaptive admission control:** accept writes at the rate the system can
  sustain, reject excess with backpressure signal.
- **(c) Let the MemTable grow:** unbounded MemTable, flush in background.
  Risk: OOM.

**Likely answer:** (a) with configurable thresholds. Following Pebble's principle
of honest stalls over silent degradation.

### 2.4 MemTable data structure (Slice 1)

**Question:** Skip list, B-tree, or something else for the sorted in-memory map?

**Options:**
- **(a) Skip list:** classic LSM choice (RocksDB, LevelDB). Lock-free concurrent
  reads + single-writer inserts. O(log n) lookup/insert.
- **(b) B-tree (like ART/adaptive radix tree):** faster point lookups, but
  harder to make concurrent and doesn't naturally produce sorted output for flush.
- **(c) Vector + sort-on-flush:** simple, excellent cache locality, but O(n log n)
  per flush and no concurrent read-during-write.

**Likely answer:** (a) skip list. Proven, well-understood, matches the LSM
pattern. Pebble uses a skip list.

### 2.5 Conflict detection granularity (Slice 2)

**Question:** Per-key tracking or interval/range-based?

**Context:** Per-key tracking has zero false conflicts but O(keys) memory.
Interval-tree tracking has some false conflicts (adjacent keys in the same
range) but O(ranges) memory, which is typically much smaller.

**Options:**
- **(a) Per-key:** exact, but memory-heavy for transactions touching many keys.
- **(b) Interval tree:** track read/write ranges. Memory-efficient. Some false
  conflicts for adjacent-but-unrelated keys.
- **(c) Hybrid:** per-key for small transactions, interval for large ones.

**Likely answer:** (b) interval tree. FDB uses this approach. Matches the
"conflict range" API design.

---

## 3. Research items still pending

| # | Item | Status | Notes |
|---|------|--------|-------|
| 1 | ~~Survey LSM research (Niv Dayan)~~ | ✅ done | Resolved by Dostoevsky [D] + Monkey [M] papers |
| 2 | ~~Dostoevsky / Monkey analysis~~ | ✅ done | Papers persisted in `docs/references/` |
| 3 | ~~Rust LSM engines (fjall, SlateDB)~~ | ✅ done | fjall [F] analyzed; SlateDB noted |
| 4 | ~~Engine landscape (10 engines)~~ | ✅ done | `docs/engine-landscape-and-ideal-path.md` |
| 5 | ~~Distributed systems analysis~~ | ✅ done | `docs/distributed-systems-analysis.md` |
| 6 | ~~FDB limitations analysis~~ | ✅ done | `docs/fdb-limitations-analysis.md` |
| 7 | ~~Why CockroachDB/TiKV didn't use FDB~~ | ✅ done | Analyzed: timing, architecture, feature gaps, language |
| 8 | RocksDB GitHub issues on compaction/amp/memtable | 🔲 open | Low priority — we have the academic analysis |
| 9 | Hardware-consciousness (NVMe, direct I/O, io_uring) | 🔲 open | Relevant for Slice 4 (SST I/O) and performance |
| 10 | Redwood (FDB's new B+tree) internals | 🔲 open | Interesting for comparison, not blocking |
| 11 | Pebble metamorphic testing framework details | 🔲 open | Relevant for Slice 8 (simulation) |
| 12 | ~~Distribution design (how embedded → distributed)~~ | ✅ done | `docs/distribution-design.md` — multi-Raft, CP, strict serializable |
| 13 | ~~Distribution deep research (Percolator, Parallel Commits, PD, TSO, Raft)~~ | ✅ done | `docs/distribution-deep-research.md` + `references/percolator-osdi2010.pdf` |
| 14 | Rust Raft implementations (openraft vs raft-rs) | 🔲 open | Research done; decision deferred to distribution layer (post-Slice 7) |
| 15 | HLC vs TSO for distributed timestamps | 🔲 open | TSO simpler; HLC scalable; both documented in deep research |
| 16 | Optimistic vs pessimistic default (distributed) | 🔲 open | TiDB switched to pessimistic for OLTP; PedraDB may want both |
| 17 | Parallel Commits implementation details | 🔲 open | Target protocol; need design when building pedradb-txn |
| 18 | In-memory vs durable distributed locks | 🔲 open | TiKV lesson: in-memory is fast, fragile under partition |

---

## 4. Design decisions already resolved

For reference — these are settled and should not be re-litigated without strong
new evidence.

| # | Decision | Source | Target |
|---|----------|--------|--------|
| 1 | Clean-room rewrite in Rust (not CXX translation) | Pebble [P1], CGO pain | Global |
| 2 | `#![forbid(unsafe_code)]` | fjall [F] | Global |
| 3 | `clippy::pedantic` with zero warnings | Code quality | Global |
| 4 | RocksDB as oracle only (not linked into core) | CXX boundary analysis | Global |
| 5 | FoundationDB layer model (TX in core, nothing else) | FDB layer concept | Architecture |
| 6 | LSM-tree (not B-tree) as storage structure | Write-heavy workload fit | Slice 4+ |
| 7 | WiscKey KV separation (values in separate log) | WiscKey [W], BadgerDB | Slice 4 |
| 8 | Monkey Bloom allocation (FPR ∝ run size) | Monkey [M] | Slice 4 |
| 9 | Lazy Leveling compaction default | Dostoevsky [D] | Slice 6 |
| 10 | `InternalKey` as struct, not encoded string | Pebble [P2] | Slice 1 |
| 11 | Fixed-size MemTable arena | Pebble [P2] | Slice 1 |
| 12 | Batch as LSM level (seqnum high-bit) | Pebble [P2] | Slice 4 |
| 13 | Commit publish-queue lock-free | Pebble [P2] | Slice 2 |
| 14 | Range tombstones integrated in merging iterator | Pebble [P2] | Slice 5 |
| 15 | Invariant-based pacing (not static rate limit) | Pebble [P2] | Slice 6 |
| 16 | No artificial write throttling | Pebble [P2] | Slice 6 |
| 17 | Static dispatch (enum+match) on iterator hot path | Pebble [P2] | Slice 5 |
| 18 | Deterministic simulation testing | FDB | Slice 8 |
| 19 | No built-in distribution (embedded first) | Architecture decision | Global |
| 20 | WAL block format compatible with RocksDB | Compatibility | Slice 0 ✅ |
| 21 | Distribution via multi-Raft (not FDB decoupled model) | Simplicity, latency, Rust Raft libs | Future layer |
| 22 | Strict serializable consistency (not eventual) | FDB model; correctness non-negotiable | Future layer |
| 23 | CP choice (consistency over availability during partition) | FDB CAP analysis | Future layer |
| 24 | Range-based sharding (not hash-based) | Preserves ordered KV semantics | Future layer |
| 25 | Single-leader per Region (not multi-master) | Strict serializability | Future layer |
| 26 | Distribution is a layer on top of embedded core | Architecture decision | Future layer |
| 27 | Cross-Region commit via Parallel Commits (not classic 2PC) | CRDB Parallel Commits | Future layer |
| 28 | Eventual consistency never as default | Foundation correctness | Future layer |
| 29 | Percolator-style primary/secondary locks for cross-Region TX | Percolator OSDI'10 + TiKV | Future layer |
| 30 | PedraDB = local library only (RocksDB role); multi-node = other product | architecture-refined.md | Global |
| 31 | Own LSM (not wrap RocksDB, not port Redwood) | architecture-refined.md | Store |
| 32 | Local TX in PedraDB (outer DB embeds it; no bolt-on from zero) | architecture-refined.md | TX |
| 33 | Distribution docs = research for outer DB, not PedraDB roadmap | architecture-refined.md | Global |
| 34 | Optimize power/surface ratio; public API ≈ open+TX CRUD+range | positioning.md | Global |
| 35 | Research LSM under the hood, not API knobs | positioning.md | Global |

---

## 5. Code-level TODOs

| Crate | Module | Item | Priority |
|-------|--------|------|----------|
| pedradb-core | — | `InternalKey` struct (`user_key`, `seqnum`, `kind`) | Slice 1 |
| pedradb-core | — | `Slice` custom value type (immutable, cheap clone) | Slice 1 |
| pedradb-core | memtable | Skip list implementation (concurrent read, single writer) | Slice 1 |
| pedradb-core | memtable | Fixed-size arena allocator | Slice 1 |
| pedradb-core | memtable | Sequence number counter (monotonic, atomic) | Slice 1 |
| pedradb-core | tx | MVCC version tracking | Slice 2 |
| pedradb-core | tx | Interval tree for conflict ranges | Slice 2 |
| pedradb-core | tx | Commit pipeline (publish-queue) | Slice 2 |
| pedradb-core | sst | Block-based SST writer/reader | Slice 4 |
| pedradb-core | sst | Monkey Bloom filter builder | Slice 4 |
| pedradb-core | vlog | WiscKey value log (append-only, addressable) | Slice 4 |
| pedradb-core | iter | Merged iterator across levels | Slice 5 |
| pedradb-core | iter | Range tombstone integration + block-skip | Slice 5 |
| pedradb-core | compact | Lazy Leveling compaction strategy | Slice 6 |
| pedradb-core | compact | Cost model (Dostoevsky equations) for auto-tuning | Slice 6 |
| pedradb-core | gc | Version GC (piggyback on compaction) | Slice 7 |
| pedradb-core | gc | Value-log GC (online, segment-based) | Slice 7 |
| pedradb-sim | — | Deterministic simulation framework | Slice 8 |
| pedradb-oracle | — | Oracle diff harness implementation | Slice 9 |

---

## 6. Documentation index

| Document | Content |
|----------|---------|
| [`architecture.md`](architecture.md) | Full architecture, anti-features, roadmap, engineering items |
| [`rocksdb-critiques-and-improvements.md`](rocksdb-critiques-and-improvements.md) | 15 design decisions from Pebble, Dostoevsky, Monkey, fjall |
| [`engine-landscape-and-ideal-path.md`](engine-landscape-and-ideal-path.md) | Comparison of 10 engines, the 3 optimizations nobody combined |
| [`distributed-systems-analysis.md`](distributed-systems-analysis.md) | ScyllaDB, Ceph, TiKV, FDB, CockroachDB, ClickHouse layer analysis |
| [`distribution-design.md`](distribution-design.md) | How embedded PedraDB becomes distributed (multi-Raft, CP, strict serializable) |
| [`distribution-deep-research.md`](distribution-deep-research.md) | Protocol-level research: Percolator, Parallel Commits, PD, TSO, Raft libs |
| [`scylladb-architecture.md`](scylladb-architecture.md) | How Scylla operates (AP multi-master, Seastar, tunable CL) vs PedraDB |
| [`tidb-architecture.md`](tidb-architecture.md) | TiDB = MySQL SQL layer on TiKV+PD+TiFlash; validates PedraDB layers |
| [`foundationdb-layers-and-products.md`](foundationdb-layers-and-products.md) | What runs on FDB: Record/Document layers, Snowflake, CloudKit, Astra, … |
| [`etcd-comparison.md`](etcd-comparison.md) | etcd vs PedraDB, FDB, TiKV/TiDB, CRDB, Scylla, RocksDB, … |
| [`competitive-landscape-rust.md`](competitive-landscape-rust.md) | Rust/local engine peers: fjall, SurrealKV, redb, AgateDB, SlateDB… |
| [`positioning.md`](positioning.md) | Focus: tiny API, speed, max layer leverage (power/surface) |
| [`compare-fjall.md`](compare-fjall.md) | PedraDB vs fjall (closest Rust peer) |
| [`rfc/0001-pedradb-high-level-spec.md`](rfc/0001-pedradb-high-level-spec.md) | **Normative** high-level RFC (P0/P1/P2, open decisions) |
| [`rfc/0001-open-decisions-deep-dive.md`](rfc/0001-open-decisions-deep-dive.md) | O1–O10 deep dive: peers, nuances, regrets |
| [`session-synthesis-architecture-and-doubt.md`](session-synthesis-architecture-and-doubt.md) | Full conversation synthesis + “is PedraDB wrong?” |
| [`grail-plan-build-databases-on-pedradb.md`](grail-plan-build-databases-on-pedradb.md) | Plan: kernel → SQLite/etcd/TiKV/TiDB-class products |
| [`pedradb-as-dcs-storage-for-patroni.md`](pedradb-as-dcs-storage-for-patroni.md) | DCS on PedraDB for Patroni elections (not etcd protocol in core) |
| [`doctrine-primitives-and-api-layers.md`](doctrine-primitives-and-api-layers.md) | Doctrine: powerful primitive + API layers only |
| [`plug-map-replace-incumbents.md`](plug-map-replace-incumbents.md) | Where to plug: etcd, Patroni, SQLite, PG, TiKV, TiDB, Scylla |
| [`fdb-limitations-analysis.md`](fdb-limitations-analysis.md) | Why PedraDB solves FDB's 4 limitations |
| [`open-items.md`](open-items.md) | This file — living status tracker |
| [`references/`](references/) | All primary sources (papers as PDF+TXT, blog posts, docs) |
