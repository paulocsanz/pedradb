# Primary extracts — Tigris architecture (2026-08-15)

Use with [`../research/object-storage/06-tigris-and-media-tiers.md`](../research/object-storage/06-tigris-and-media-tiers.md)
and [`../research/object-storage/07-montanha-as-tigris-control-plane.md`](../research/object-storage/07-montanha-as-tigris-control-plane.md).
This file is quotes and measured claims from pages fetched this session, not a product decision.

## Official architecture

Source: <https://www.tigrisdata.com/docs/concepts/architecture/> (fetched 2026-08-15)

- Deployment: **API gateways** + **cache** + **data distribution/replication** + **data and metadata storage**.
- Gateway: **S3 API**, authz, routing. **Stateless** workers, many regions, request close to the user.
- Cache: nodes in every gateway region. **Cache on Read** (default). **Cache on Write** (eager, per-bucket).
- Metadata (object location, user meta, buckets, users/orgs, IAM): **separate layer**, **transactional**, in **FoundationDB**.
  They name CAS, multi-object TX, rich query as reasons FDB exists here — “none of which is provided by S3.”
- FDB: ordered KV, **multi-key strictly serializable transactions across the entire keyspace**, Spanner-class consistency, simulation testing.
- Sharding: lexicographic keys; object key → logical shard → physical node.
- Multi-cluster: **multiple FDB clusters**; a **replication service they wrote** copies metadata between clusters (region-wide failure).
- Object **bytes**: **block stores**, closest to the user. Metadata extracted first; content goes to the block store.
- Distributor: **distributed persistent queue backed by FDB**, “adaptation of Apple’s QuiCK.” Jobs: FDB-to-FDB replication, geo cache, cache invalidate on write, extra copies if requested, move object.

**Not on this page:** Reed-Solomon profile, erasure set, CRUSH, HDD rebuild of a 16 TB disk.

## Small objects

Source: <https://www.tigrisdata.com/blog/benchmark-small-objects/> (2025-07-08, fetched 2026-08-15)

- Accelerates small objects by (i) **inlining very small objects inside metadata records**, (ii) coalescing adjacent keys, (iii) on-disk **LSM-backed cache**.
- Their YCSB (10 M × 1 KB, then 1 M ops 80/20): Tigris load p50 PUT **16.8 ms** / p90 **35.9 ms**; mixed read p50 **5.4 ms** / p90 **7.9 ms**. Vendor bench on Oracle SJC; treat as their number, not ours.

Source: Fly public beta <https://fly.io/blog/tigris-public-beta/> (2024-02-15)

- “Redundant FoundationDB clusters in our regions to **track objects**.”
- “Fly.io’s **NVMe volumes** as a first level of **cached raw byte store**.”
- QuiCK-modelled queue to “multiple replicas, to regions where the data is in demand, and to **3rd party object stores… like S3**.”
- “If your objects are **less than about 128 kilobytes**, Tigris makes them **instantly global**.”

Source: Fly customer story <https://fly.io/customers/tigris/>

- “Taking advantage of core FoundationDB features, Tigris **‘inlines’ small objects in its metadata storage**, storing object bytes right alongside routing information.”
- “For small objects, Tigris can achieve performance **comparable to Redis**, with full consistency and replicated storage.”
- “Redundantly storing data directly across our **NVMe volumes** as well as **off-network object storage** peered to Fly.io’s networks.”

Source: Fly community (Tigris staff, 2024-03-10) <https://community.fly.io/t/tigris-not-on-fly-io/18672>

- Everything except **large object storage** runs on Fly. Large objects: Fly storage **or** remote S3/OCI by proximity. Users never talk to the remote store.

## Snapshots / forks (metadata feature, not EC)

Source: <https://www.tigrisdata.com/blog/bucket-forking-deep-dive/> (2025-11-06, fetched 2026-08-15)

- Snapshot = **one u64**: `MaxUint64 - unix_nanos(UTC)` (they also describe it in prose as “nanoseconds since 1970”; the **code they print** is the inverted value).
- Key: `bucket-name/object-name/version-id-timestamp`. FDB ordered. Newest sorts first because time is encoded backwards.
- “This effectively turns each Tigris object into its own write-ahead log.”
- Read at snapshot: newest version whose inverted stamp is still ≥ snapshot (newest timestamp that came **before** the snapshot).
- Fork: child bucket; miss recurses into the parent snapshot; tombstone stops the walk. **No copy of terabytes.**
- Caveats they publish: existing buckets cannot be snapshot-enabled yet; snapshot buckets are Standard-tier only; no lifecycle/TTL yet.

Source: <https://www.tigrisdata.com/docs/snapshots-and-forks/> (fetched 2026-08-15)

- Snapshots O(1) (references, not bytes). Forks share until write. Readers don’t block writers.

## What is still unpublished (do not invent)

- Reed-Solomon k+m, crush map, “we rebuild a 16 TB HDD.”
- Exact inline cutoff in the official architecture page (Fly said **~128 KiB** for instantly-global; the small-object blog says “very small” inside the metadata record).
- Media of their Glacier-class restore (~1 h). Not fetched as a primary this session.
