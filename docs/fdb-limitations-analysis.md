# FoundationDB limitations: root-cause analysis and how PedraDB resolves them

> FoundationDB is the design model PedraDB follows — ACID transactions in the
> core, layers on top. But FDB has well-known operational limitations. This
> document analyzes each one, identifies its **root cause**, and explains why
> an embedded engine like PedraDB does (or doesn't) inherit it.

## TL;DR

Every FDB design limitation is a consequence of its **distributed client-server
architecture**, not a fundamental law. An embedded engine that does transactions
locally — where the "client" and the "storage server" are the same process —
sidesteps them entirely. The **only** thing that remains is OCC conflict rate
for long-running concurrent transactions, which is a property of optimistic
concurrency control itself, not of any particular deployment topology.

| FDB limitation | Value | Root cause | PedraDB inherits it? |
|---------------|-------|------------|----------------------|
| Transaction timeout | 5 s | Network round-trips, remote resolvers, replication latency | ❌ No — local commit, no network |
| Transaction size limit | 10 MB | Serialized over network; resolver must hold all read/write ranges in memory | ❌ No — no resolver, no serialization |
| Value size limit | 100 KB | Every value replicated 3× and served through proxies | ❌ No — WiscKey value log handles large values natively |
| No long-running transactions | Conflicts with resolver GC | Resolver memory bounded; MVCC versions garbage-collected aggressively | ❌ No — local MVCC, GC tied to active snapshots |

---

## The architecture difference that matters

```
FoundationDB (distributed)              PedraDB (embedded)

┌──────────┐   network   ┌───────────┐   function call   ┌──────────┐
│  Client  │ ──────────→ │  Commit   │ ────────────────→ │  Storage │
│  (app)   │             │  Proxy    │                   │  Engine  │
│          │ ←────────── │  Resolver │ ←──────────────── │  (LSM)   │
└──────────┘   network   └───────────┘   function call   └──────────┘

Transaction crosses:                    Transaction crosses:
  1. Client → Proxy (network)             1. App → Engine (function call)
  2. Proxy → Resolver (network)           That's it.
  3. Resolver → Storage (network)
  4. Storage → Replicas (network)
  5. Reverse path for response
```

FDB's commit path involves **5+ network hops** per transaction. The 5-second
timeout exists because each hop adds latency, and holding locks or resolver
state for longer than a few seconds is untenable at cluster scale. PedraDB's
commit path is a **function call** — the transaction manager and the storage
engine live in the same process, sharing memory.

---

## Limitation 1: 5-second transaction timeout

### What FDB does
Every transaction must commit or abort within 5 seconds. If it doesn't, FDB
forcibly aborts it with `transaction_too_old`.

### Root cause: distributed commit latency
The timeout is not arbitrary — it's the maximum time FDB can guarantee a
consistent snapshot is available across:

- **Commit proxies** (which assign commit versions)
- **Resolvers** (which check conflicts)
- **Storage servers** (which hold the data)
- **Transaction logs** (which durably persist writes before they're visible)

Each role may be on a different machine. The 5-second window is the time the
system guarantees MVCC versions remain available for reads. After that, old
versions are garbage-collected to bound memory usage cluster-wide.

### Why PedraDB doesn't inherit this
PedraDB is embedded — there is no network between the application and the
storage engine. A transaction's read snapshot and commit path are entirely
local:

- **No commit proxy round-trip:** the sequence number is assigned locally.
- **No resolver round-trip:** conflict detection happens in-process.
- **No replication round-trip:** durability is local (WAL + fsync).

A PedraDB transaction can remain open for seconds, minutes, or hours, as long
as the application holds the snapshot. The only cost is retaining MVCC versions
that are newer than the snapshot — bounded by available memory, and controllable
via configuration.

### Trade-off that remains
Long-running transactions retain old MVCC versions, preventing version GC.
PedraDB needs a **version GC strategy** (see open items) that can either:
- Reclaim versions older than the oldest active snapshot (stop-the-world pause
  risk if a snapshot is very old).
- Implement a "snapshot is too old" error (like FDB) as a safety valve.

---

## Limitation 2: 10 MB transaction size limit

### What FDB does
A single transaction's total writes cannot exceed 10 MB. Read ranges are also
bounded (though can be paginated).

### Root cause: network serialization + resolver memory
The entire transaction (all keys written, all read ranges for conflict
detection) is serialized and sent over the network to commit proxies and
resolvers. The resolver must hold all read/write ranges in memory to perform
conflict detection against concurrent transactions. A 100 MB transaction would
require the resolver to hold 100 MB of range data per concurrent transaction —
untenable when thousands of transactions are in-flight.

### Why PedraDB doesn't inherit this
There is no serialization, no network, and no separate resolver:

- **Writes go directly to the MemTable:** the transaction's mutations are applied
  in-process to the MemTable (or a flushable batch if too large). No serialization.
- **Conflict detection is local:** the read ranges are checked against local
  version metadata. Memory for conflict tracking is process memory, not a
  separate server's bounded allocation.
- **Large transactions become flushable batches:** following Pebble's design
  (see `rocksdb-critiques-and-improvements.md` §5), a transaction too large for
  the MemTable arena is promoted to a flushable batch — sorted once, added to the
  immutable MemTable list, readable as an LSM level. No OOM, no death loop.

### Practical limit
The limit becomes available memory, not an arbitrary constant. A machine with
32 GB RAM can handle a multi-GB transaction if needed. The flushable batch
mechanism ensures this degrades gracefully rather than crashing.

---

## Limitation 3: 100 KB value size limit

### What FDB does
Individual values are capped at approximately 100 KB (`VALUE_SIZE_LIMIT`).
Larger values must be split across multiple keys by the application.

### Root cause: replication and proxy overhead
Every value written to FDB is:
1. Serialized by the client
2. Sent to a commit proxy
3. Written to a transaction log
4. Replicated to storage servers
5. Replicated again for fault tolerance (typically 3× total)

A 1 MB value means 3 MB of network traffic and storage per write. Large values
also skew compaction and increase the cost of range reads. The 100 KB limit is
a pragmatic engineering decision to keep the distributed system manageable.

### Why PedraDB doesn't inherit this
PedraDB adopts **WiscKey KV separation** as a first-class design principle
(see `engine-landscape-and-ideal-path.md`):

- **Keys** go through the LSM tree (small, compacted, Bloom-filtered).
- **Values** go to a separate append-only **value log** (written once, never
  compacted unless garbage-collected).

This means a 10 MB value is written once to the value log, and its location
pointer (a few bytes) goes through the LSM. The LSM tree stays compact
regardless of value sizes. Point lookups fetch the value via one random read
to the value log — an SSD operation taking ~100 μs.

### Trade-off that remains
Large-value range scans require random reads to the value log (one per value),
which is slower than sequential reads from SSTs. This is the fundamental
WiscKey trade-off, well-characterized in the original paper. For workloads
dominated by large-value range scans, a hybrid mode (inline small values, log
large values) can be offered as a configuration knob.

---

## Limitation 4: No long-running read/write transactions

### What FDB does
FDB transactions are designed to be short-lived. Long-running transactions are
problematic because:
- The 5-second timeout kills them.
- Even if retried, they conflict frequently with concurrent writes (OCC).
- They prevent version GC, growing resolver and storage memory.

FDB recommends "pipelining" long operations: break them into chunks, each in
its own short transaction.

### Root cause: resolver memory + MVCC GC + network
The resolver must track all concurrent transaction conflicts. Long-running
transactions hold a read snapshot version, preventing MVCC GC. At distributed
scale, this affects the entire cluster's memory budget, not just one node.

### Why PedraDB doesn't inherit this (mostly)
An embedded engine has fundamentally different constraints:

- **No resolver memory budget to exhaust:** conflict tracking is local process
  memory, which scales with the machine, not with cluster size.
- **MVCC versions are local:** old versions live in the MemTable / SSTs. GC
  reclaims them during compaction (or explicitly). The cost of retaining old
  versions is bounded by local disk, not distributed memory.
- **No network timeout:** a transaction can stay open as long as the process
  lives. Reads against an old snapshot scan MemTable + SSTs filtered by
  sequence number — O(read) cost, not O(transaction-duration) cost.

### The one thing that DOES remain: OCC conflict rate
If a long-running transaction reads key K at time T₁, and another transaction
writes K at time T₂, the long transaction will abort on commit (optimistic
concurrency control detects the conflict). This is **not** a distributed-system
artifact — it's a property of OCC itself. It applies equally to FDB, PedraDB,
and any system using optimistic concurrency.

Mitigations (not solutions — this is fundamental):
- **Range-based conflict tracking:** only abort if the exact key or overlapping
  range was written, reducing false conflicts.
- **Serializable snapshot isolation (SSI):** detect serialization conflicts
  early (read-only anomalies), reducing wasted work.
- **Adaptive concurrency:** allow the application to declare conflict ranges
  explicitly, trading precision for abort rate.

This is the only FDB limitation that PedraDB shares — and it's inherent to
optimistic concurrency control, not to any particular architecture.

---

## Summary: PedraDB's position in the design space

```
                    ACID Transactions
                          │
           ┌──────────────┼──────────────┐
           │              │              │
      Embedded?      Distributed?    Both?
           │              │
     ┌─────┴─────┐  ┌────┴────┐
     │           │  │         │
  RocksDB     FDB  Cockroach  TiKV
  (no TX)    (TX)  (TX layer) (TX layer)

  PedraDB sits here:
  ┌─────────────────────────────┐
  │  Embedded + ACID TX  │  ← nobody occupies this
  └─────────────────────────────┘
```

**RocksDB** is embedded but has no core transactions.
**FoundationDB** has transactions but is client-server (inherits distributed
limitations).
**CockroachDB** and **TiKV** are distributed with bolted-on transaction layers.

PedraDB occupies the empty intersection: **embedded + ACID transactions**.
It gets FDB's transactional model without FDB's distributed-systems tax. The
only inherent limit it shares is OCC conflict rate for long concurrent
transactions — a property of the concurrency control method, not the
architecture.

---

## Other official API / design limitations (beyond the big four)

Source: [Known Limitations](https://apple.github.io/foundationdb/known-limitations.html) and
[Anti-Features](https://apple.github.io/foundationdb/anti-features.html) (FDB 7.3 docs).

### Still design / API limits (not only “the big four”)

| Limitation | Official detail | PedraDB inherits? |
|------------|-----------------|-------------------|
| **Key size ≤ 10,000 bytes** | Hard client error if exceeded | Soft: can allow larger keys; still should keep keys small for LSM/range efficiency |
| **Value size ≤ 100,000 bytes** | Hard limit (already analyzed) | ❌ WiscKey |
| **Transaction ≤ 10 MB affected data** | Keys/values/ranges written **and** keys/ranges **read** count; values *read* do **not** count | ❌ local memory bound |
| **TX duration ≤ 5 s** | After first read: further DB reads → `transaction_too_old`; commit with writes → abort | ❌ local MVCC |
| **Key selectors with large offsets are O(offset)** | Paging via `key+1000` style offsets is a mis-pattern | Same algorithmic issue if we expose offset selectors; fix = limit+resume / iterators (FDB’s own workaround) |
| **Not a security boundary** | Anyone who can connect can read/write **all** keys; no user-level ACL in core | Same if we keep FDB-style minimal core — ACL is a layer or process isolation |
| **Hot-key read ceiling** | Reads load-balance across replicas, but RF is fixed (e.g. 3) → ~aggregate of 3 processes for one hot key/range | Embedded: one process; distributed multi-Raft: same class of hot-key problem (single leader) |
| **No indexes / SQL / query language in core** | Deliberate anti-feature | ✅ same by design (PedraDB anti-features) |
| **No disconnected / offline-first in core** | Would sacrifice ACID | ✅ same — app/layer problem |
| **No long R/W transactions (by design)** | MVCC conflict window | Partially: duration ok embedded; OCC aborts remain |

### Soft / operational limits (current version, not pure API)

| Limit | Official note | PedraDB relevance |
|-------|---------------|-------------------|
| **Transactions > 1 MB** | “Should modify design”; can hurt performance/availability briefly even under 10 MB cap | Careful with huge embedded TX too (memtable/flushable batch) |
| **Cluster size ~500 processes** tested | Larger may scale sublinearly | Future distribution concern |
| **DB size tested ~100 TB** logical KV | Disk much larger with replication | Future scale testing |
| **HDD + SSD engine** | Rotational disks not recommended for SSD engine | Ops, not API |
| **Must retry on conflicts / `commit_unknown_result`** | Client responsibility (OCC + uncertain commit) | Same class of API for distributed; embedded can simplify some cases |

### API surface realities (not always listed as “limitations” but constrain design)

These are part of the **minimal KV API contract**, not bugs:

1. **Only ordered KV operations** — get, set, clear, range, atomic ops, watches in some bindings; no joins, no secondary indexes.
2. **Optimistic concurrency only** — no server-side row locks like InnoDB; conflicts = abort + client retry.
3. **Client is in the loop** — reads go to storage servers; writes buffer on client until commit (FDB model). Layers implement richer models in process.
4. **Conflict ranges are explicit or inferred from reads/writes** — snapshot reads intentionally skip conflict tracking (you must add ranges if needed).
5. **System keyspace `\xff`** — reserved; layers must not corrupt system metadata.
6. **Watches are not a full CDC product** — useful, but not changefeeds/SQL triggers.
7. **Language bindings vary** — C API is the core; higher-level features differ by binding.

### What PedraDB still does **not** magically remove

Even with embedded solving 5s / 10MB / 100KB / long TX:

| Remains | Why |
|---------|-----|
| OCC abort under contention | Concurrency control method |
| Need for client retry logic | Same for any OCC system |
| Hot keys as throughput ceiling (when distributed) | Single-leader per key range |
| No SQL/indexes in core | Deliberate (same anti-features as FDB) |
| Security not free | Process isolation / layer ACL |
| Bad key design (huge keys, offset paging) | Algorithmic / modeling |

---

## References

| Ref | Source |
|-----|--------|
| [FDB-lim] | apple.github.io/foundationdb/known-limitations.html (2026-08-11) |
| [FDB-anti] | apple.github.io/foundationdb/anti-features.html (2026-08-11) |
| [FDB-layer] | FoundationDB layer concept — `docs/references/foundationdb-layer-concept.md` |
| [W] | WiscKey (FAST'16) — `docs/references/wisckey-fast2016.pdf` |
| [P2] | Pebble vs RocksDB differences — `docs/references/pebble-vs-rocksdb-differences.md` |
| Prior analysis | `docs/distributed-systems-analysis.md` (FDB section) |
