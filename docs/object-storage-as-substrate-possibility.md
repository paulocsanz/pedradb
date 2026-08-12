# Object storage as a substrate — a possibility, with nuances

> **Superseded in scope (2026-08-12).** This note only covers
> SlateDB / WarpStream / turbopuffer / Tigris and a kernel-vs-WAL-export
> question. It **misses** the live SQLite-VFS + one-DB-per-agent category
> (Rivet, Turso AgentFS/diskless, mvSQLite/Willow, Litestream VFS) and it
> **gets Tigris’s data plane wrong** (FDB is metadata; object bytes live on
> block stores — see Tigris’s own architecture page). SST/compaction are
> also no longer “not built yet.”
>
> Canonical follow-up (primaries + code, not this summary):
> [`sqlite-object-storage-agents-and-pedradb.md`](sqlite-object-storage-agents-and-pedradb.md).
> The Rung-0 kernel exclusion still holds; do not use *this* file as the
> map of the object-storage trend.

> **Status: exploratory, not a decision.** `positioning.md` currently lists
> "Object-store-first (S3)" as a permanent non-goal "until proven wrong,"
> citing SlateDB/Tonbo as the occupants of that niche. This doc takes that
> seriously and asks the question properly: given the platform-wide move
> toward treating block storage and object storage as distinct, first-class
> infra primitives (Railway Volumes vs Buckets; Tigris on Fly; the same
> split shows up everywhere), does object storage deserve a role somewhere
> in the grail ladder — not as the kernel's medium, but as a medium for one
> of the rungs above it? Short answer up front: **no for Rung 0, plausibly
> yes for Rung 1.5 and a new speculative rung above SST/compaction — with
> real, load-bearing nuances that make this young, not free.** Full
> reasoning below.

Primary sources (fetched 2026-08-11):
- [SlateDB — design overview](https://slatedb.io/docs/design/overview/)
- [SlateDB RFC-0001 — Manifest design](https://slatedb.io/rfcs/0001-manifest/)
- [WarpStream — "Zero Disks is Better (for Kafka)"](https://www.warpstream.com/blog/zero-disks-is-better-for-kafka)
- [turbopuffer — Architecture](https://turbopuffer.com/docs/architecture)
- [turbopuffer — Tradeoffs](https://turbopuffer.com/docs/tradeoffs)
- [Tigris — Architecture](https://www.tigrisdata.com/docs/concepts/architecture/)
- [Tigris — "What's the big deal with conditional writes support in S3?"](https://www.tigrisdata.com/blog/s3-conditional-writes/)
- [AWS — S3 now delivers strong read-after-write consistency (Dec 2020)](https://aws.amazon.com/about-aws/whats-new/2020/12/amazon-s3-now-delivers-strong-read-after-write-consistency-automatically-for-all-applications)
- [AWS Storage Blog — Building multi-writer applications on S3 using native controls](https://aws.amazon.com/blogs/storage/building-multi-writer-applications-on-amazon-s3-using-native-controls/)
- [Railway docs — Volumes reference](https://docs.railway.com/volumes/reference), [Storage Buckets](https://docs.railway.com/storage-buckets), [Data & Storage](https://docs.railway.com/data-storage)
- In-repo: `positioning.md` (existing non-goal), `competitive-landscape-rust.md` §4.1 (SlateDB), `engine-landscape-and-ideal-path.md` (SlateDB), `sql-lessons-for-the-grail.md` (Neon's cold object-storage tier), `foundationdb-layers-and-products.md` (Tigris on FDB)

---

## 1. The platform-trend observation, grounded in real docs

The question came from watching Railway (block storage), an internal
platform ("elastic storage"), and Fly (Tigris) all converge on the same
split. Worth confirming this is real, not vibes — Railway's own docs draw
the line sharply:

| | **Railway Volumes** (block) | **Railway Buckets** (object) |
|---|---|---|
| Model | Attach to exactly **one** service | S3-compatible API, any number of clients |
| Replicas | **"Replicas cannot be used with volumes"** (documented caveat) | Inherent — S3 semantics are multi-reader by design |
| IOPS | Fixed **3,000 read / 3,000 write**, same at every size | Not IOPS-shaped; billed per request + per GB |
| Ceiling | Self-serve to 1 TB, then Enterprise | No documented ceiling |
| Failure mode | "small amount of downtime" on redeploy — single mount point is a real constraint, not a formality | N/A — stateless clients |

That single documented caveat — *replicas cannot be used with volumes* — is
the whole thesis of this doc in one sentence: block storage's scaling model
runs out exactly where "elastic" and "distributed" begin, and object
storage is where the industry keeps landing instead.

**Tigris (the thing "Fly has too") is not raw S3 underneath — and it is
also not “FDB storing the objects.”** Its own architecture page splits
the plane: **metadata** (location, user meta, buckets, IAM) lives in
**FoundationDB** (strict serializability, CAS, multi-key TX, multi-cluster
replication); **object bytes** go to regional **block stores**. The S3
costume is the API + metadata TX, not the data plane. The layer pattern
still holds for *metadata* (same family as Snowflake-on-FDB); do not
round that up to “S3 products are secretly FDB.” See the 2026-08-12
correction in
[`sqlite-object-storage-agents-and-pedradb.md`](sqlite-object-storage-agents-and-pedradb.md).

### Why this became possible only recently

Two AWS changes are the entire reason "build a real database directly on
S3" went from a bad idea to a product category in about four years:

| Year | Change | What it unlocked |
|---|---|---|
| **2020** | S3 **strong read-after-write consistency**, automatic, for all requests | Before this, a `PUT` might not be visible to the next `GET`/`LIST` — building anything correctness-sensitive on S3 required your own workaround. After: reads see writes immediately, no caveat |
| **2024** | S3 **conditional writes** (`If-None-Match` / `If-Match` on `PutObject`) | Single-key compare-and-swap **without a side lock service**. This is what lets a manifest/pointer file be updated safely by exactly one racing writer |

Before 2024, SlateDB's own docs describe the workaround for S3 specifically
(GCS and Azure already had native CAS): write to a temp location, record
intent in **DynamoDB**, copy to final destination, mark complete — a whole
extra coordination service bolted on *just* to fake compare-and-swap. That
workaround existing in a production system's docs, even as a footnote, is
itself evidence of how new and unsettled this ground still is.

---

## 2. What's actually been built on it — three systems, with real numbers

### SlateDB — the closest analogue to PedraDB itself

Full LSM tree, same shape as PedraDB (WAL → MemTable → SST → compaction →
manifest), except **every one of those lives as objects**, including the
WAL:

- **WAL isn't written per-`put()`.** Writes batch into the WAL by
  `flush_interval`, `flush_bytes`, or `skip_memtable_bytes` — multiple
  key-value pairs share one WAL-SST object. This is non-negotiable: object
  storage is billed and latency-bound *per request*, so batching isn't an
  optimization, it's the precondition for the whole idea working at all.
- **Single-writer via epoch fencing in the manifest**, not Raft. On
  startup a writer bumps `writer_epoch` and writes a fencing SST; any older
  ("zombie") writer that tries to write next finds a higher epoch already
  there and halts. Deterministic, no consensus protocol, no heartbeats —
  just conditional-write semantics on one object.
- **Manifest stays small at real scale**: "100,000 compacted SSTs and 1,000
  snapshots" was measured at **~5.6 MB**. The control-plane object doesn't
  become the bottleneck even with a large data footprint.

### WarpStream — Zero Disk Architecture, for a log instead of a KV store

Kafka-protocol-compatible streaming with **zero local disks anywhere** —
"agents" (their broker equivalent) are fully stateless; every byte goes
straight to object storage. Explicit trade, in their own words: **"a little
extra latency"** in exchange for **>24x lower storage cost** and trivial
autoscaling (stateless agents can be added/removed with no data to
rebalance). They name the workloads this is wrong for only implicitly:
anything needing single-digit-millisecond latency (their own framing is
"99% of use-cases" tolerate the trade — the other 1% doesn't).

### turbopuffer — the most quantified of the three

This is the one with numbers precise enough to actually build a mental
latency budget from:

| Path | Latency | Mechanism |
|---|---|---|
| **Write commit** | up to **200ms**, group-committed **at most once/sec** | WAL batched to object storage, not per-request |
| **Cold read** (not cached) | **p50 = 874ms** for 1M documents | 3–4 object-storage round trips: fetch centroid index, locate nearest centroids, fetch clusters |
| **Warm read** (cached on local NVMe) | **p50 = 14ms** | Read-through cache; object storage is source of truth, NVMe is the hot tier |
| **Consistent read floor, even warm** | **~10ms** | Still has to check object-storage metadata for freshness — this floor doesn't go away with caching |
| **Eventual-consistency mode** | lower latency, but | up to **~1 hour** staleness in the worst case, traded explicitly for speed |

turbopuffer's own tradeoffs page states the constraint bluntly: object
storage imposes **a ~10ms latency floor** on consistent reads, full stop —
not an engineering gap, a property of the medium.

---

## 3. The mechanism everyone converges on (name it once, clearly)

Every system above — and Neon's Pageserver/object-storage split from
`sql-lessons-for-the-grail.md` — is the same three-part trick:

1. **Bulk data as immutable objects** (SSTs, log segments, index shards) —
   written once, never mutated, cheap to replicate/cache because nothing
   about them ever changes after the write.
2. **One small, frequently-rewritten pointer object** (manifest, registry,
   metadata file) that says which bulk objects are current — updated via
   **conditional PUT** so exactly one racing writer's update survives. This
   is the entire "distributed coordination" story: no Raft, no lock
   service, just CAS on one object.
3. **A local cache in front of it** (NVMe, RAM) for anything that needs to
   answer faster than an object-storage round trip — because nothing about
   part 1 or 2 changes the physics of part 3 being necessary.

And two hard limits nothing here removes, confirmed directly from the AWS
docs on conditional writes:

- **Atomicity is per-object, not cross-object.** Conditional writes are
  single-key CAS. No native multi-object transaction exists in plain S3 —
  which is exactly why Tigris had to put FoundationDB underneath to offer
  one, and why SlateDB's manifest is deliberately designed as *one* object
  everything else references, never spread across several that would need
  to move together.
- **Latency floor in the tens-to-hundreds of ms**, not microseconds — this
  is network-round-trip-plus-provider-metadata-lookup, and no client-side
  cleverness removes it for the *first* request to any given piece of data.

---

## 4. Fit against PedraDB's own thesis — where this could actually slot in

PedraDB's identity is in-process, sub-millisecond commit, mandatory
multi-key ACID. Object storage's floor (10–200ms per round trip, per §2) is
**100–1000x** higher latency than that target. This is not a "needs more
engineering" gap — it's the same physics turbopuffer's own docs state
outright. So:

**Rung 0 (kernel) stays object-storage-free.** The existing non-goal in
`positioning.md` is **confirmed, not overturned**, by this research — every
system studied pays this latency only on a write/cold-read path that is
allowed to be slow, never on the path PedraDB's whole pitch depends on.

But two places in the grail ladder look genuinely different after this
research:

### (a) Rung 1.5 — object storage as an alternative medium for WAL export

Last session's grail-plan edit added **Rung 1.5: WAL-shipped read
replicas** (single writer, async WAL frames to followers, no consensus) and
promoted **WAL/seq-number export to Must**, precisely because the
Aurora/Neon research showed the log is the real distribution primitive.
This research adds a concrete, already-proven option for *where* those WAL
frames go: **object storage, instead of (or in addition to) a private
network to another PedraDB process.** SlateDB is a working existence proof
in Rust that "WAL frames as batched objects + a CAS-fenced manifest" is a
buildable, real mechanism — not a hypothetical. The payoff: bottomless,
cross-region-replicated durability and the ability to spin up a read
replica *anywhere* an object-storage endpoint is reachable, without writing
custom replication — at the cost of exactly the latency this doc documents,
paid only on the export path, never on the local commit path.

### (b) A new, more speculative rung: PedraDB as the hot cache in front of an object-storage source of truth

This generalizes the Aurora/Neon Pageserver lesson using the vocabulary
this doc just built: object storage holds the durable, bottomless history;
**PedraDB — local, in-process, ACID — plays the role turbopuffer's NVMe
cache node or Neon's Pageserver plays**, materializing the hot working set
so reads never pay the object-storage floor. This is real (turbopuffer
proves the shape works at production scale). **SST/compaction/vlog now
exist** (`sst/table.rs` v4, `Db::compact`, `compact_vlog`) — the 2026-08-11
“not built yet” clause is stale. The remaining hole is an object-shaped
media trait (PUT/GET/CAS), not the LSM itself. Still Rung 4/5-adjacent.

### Where this does not change anything

The single-writer-per-manifest pattern (SlateDB's epoch fencing) is the
**same constraint** the grail plan already committed to — single writer
per key range, no multi-master (§9, §10 of `grail-plan-build-databases-on-pedradb.md`).
Object storage doesn't relax that; it just moves *where* the
fencing/leader-election happens, from a Raft term into a conditional PUT.
That's a confirmation of an existing decision, not a new one.

---

## 5. Nuances and risks — the part that matters as much as the opportunity

- **Not every object store gives you the CAS you need.** SlateDB's own
  workaround for pre-2024 S3 (a DynamoDB side-lock, per §1) is a warning:
  if PedraDB ever exposes this, it must detect/require native conditional
  writes (S3 ≥ Aug 2024, GCS, Azure Blob) rather than assume every
  "S3-compatible" endpoint has them — some self-hosted MinIO-class targets
  and older S3-compatible clones may not.
- **Billing and latency are per-request, not per-byte.** PedraDB's own
  natural default (commit-per-transaction) would be financially and
  latency-wise disastrous pointed unmodified at object storage — every
  system studied here batches aggressively specifically to amortize this
  (SlateDB's `flush_interval`/`flush_bytes`, turbopuffer's once-per-second
  group commit). Any object-storage-backed rung needs that batching layer
  built in from the start; it cannot be "just point the WAL writer at S3."
- **Consistency is real, but scoped to one store.** Strong read-after-write
  consistency (2020) is a guarantee *within* a given object store's own
  boundary. Cross-region or cross-provider replication (Tigris's own
  multi-cluster FDB sync, or S3 Cross-Region Replication) is asynchronous
  and reintroduces a staleness window — "object storage is now strongly
  consistent" does not mean "globally consistent across regions for free."
- **This is young ground, not decade-proven.** SlateDB is pre-1.0 with
  features explicitly marked "planned" in its own docs
  (`engine-landscape-and-ideal-path.md` already flags this). The
  pre-2024-S3 CAS workaround existing as recently-relevant documentation is
  itself a sign the ecosystem's plumbing is still being finished, unlike
  local-disk LSM (RocksDB) or network WAL shipping (MySQL binlog), both
  decades-proven.
- **It's a real dependency, not a free upgrade.** Adopting object storage
  for the WAL means crash recovery, backup, and "durable" now depend on a
  third-party API's SLA, latency, and pricing — not just the local
  filesystem's `fsync`. That's a philosophical shift from "boring local
  kernel, nothing external to trust" to "kernel with an outsourced disk."
  Worth being explicit about even while treating the possibility seriously.

---

## 6. Recommendation (as a possibility, not a decision)

| Rung | Verdict | Why |
|---|---|---|
| **Rung 0 (kernel)** | **Stays excluded** — `positioning.md`'s non-goal confirmed | Latency floor is 100–1000x PedraDB's own target; every system studied pays this cost only where it's allowed to, never on the sub-ms commit path |
| **Rung 1.5 (WAL export)** | **Worth a real experiment** | Reuses the already-Must WAL/seq-export primitive; SlateDB proves the batching + CAS-fenced-manifest mechanism works in Rust today; low blast radius (opt-in export target, not a core rewrite) |
| **New rung: local cache over object-storage source of truth** | **Real but long-horizon** | Needs SST/compaction to exist first; proven at scale by turbopuffer/Neon, but not close to PedraDB's current state |

This doc does not change any `Must`/`Must not` table — it adds an
**explicit, tracked, open possibility** to the grail plan's decision log
(see below), distinct from the settled Rung-0 non-goal so the two don't get
conflated in future sessions.

---

## Proposed link back into the grail plan and positioning docs

- `positioning.md`'s non-goal row "Object-store-first (S3) | Different
  niche (SlateDB/Tonbo)" should gain a footnote: *kernel exclusion
  confirmed; WAL-export-medium question is open, tracked separately in
  `object-storage-as-substrate-possibility.md`* — so a future reader doesn't
  read the non-goal as "we never looked at this again."
- `grail-plan-build-databases-on-pedradb.md` §12 (decision log) gets one new
  row: *"Object storage for Rung 1.5 WAL export"* → **open possibility, not
  decided** — SlateDB-proven mechanism (batched WAL objects + CAS-fenced
  manifest), real latency/cost/consistency nuances documented, revisit when
  someone actually needs cross-region durability more than sub-ms commit.
