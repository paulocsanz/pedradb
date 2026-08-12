# Primary extracts — SQLite / object storage / agents (2026-08-12)

Fetched this session. Quotes are from the live pages, not second-hand
summaries. Use with `docs/sqlite-object-storage-agents-and-pedradb.md`.

---

## Rivet — Zero-disk S3-tiered SQLite (2026-07-31)

Source: <https://rivet.dev/blog/2026-07-31-how-we-built-the-first-zero-disk-s3-tiered-storage-engine-for-sqlite/>

- Every Actor = isolated SQLite. Target: millions of DBs, instant start, idle
  costs only storage, no local disk on compute.
- Eight constraints: (1) durable writes in low ms with ≥3-way redundancy;
  (2) instant open, no restore; (3) zero-disk compute; (4) self-host, no
  proprietary cloud lock-in; (5) no new stateful DB to operate; (6) bottomless
  per DB (explicitly vs DO/D1 **10 GB cap**); (7) billions of DBs/cluster;
  (8) idle DB uses no live resources.
- Rejected: Litestream/LiteFS (primary holds full file; async ship loses
  recent writes if machine dies). rqlite/dqlite (full copy/node). Turso
  Partial Sync (still a local file). **SlateDB** (commit waits on object PUT,
  50–100 ms S3 Standard; per-DB manifest/compaction kills billions-of-DBs;
  pre-1.0). **mvSQLite** (closest; pre-1.0; operate FDB+mvstore; no S3 tier).
- Design: SQLite VFS (`xRead`/`xWrite`/`xSync`) → hot tier (RocksDB **or**
  Postgres **or** FoundationDB). S3 never on the write path. Pages 4 KiB
  grouped into **256 KiB chunks (64 pages)** for S3 offload. Compactor moves
  chunks idle 7 days (configurable) to S3; read rehydrates.
- Cost table in post: 3× EBS gp3 $0.24/GB-mo vs S3 $0.023; 5 TB → $1,230 vs
  $118. S3 Standard R/W ~30–45 / 30–100 ms; EBS ~1 ms.
- Single-writer: Actors enforce it structurally. Dual-write guard as fallback.
- vs Durable Objects: DO pinned to create DC, SQLite on same machine, CF
  pre-provisions; Rivet self-host must start any Actor on any machine.
- vs FUSE: needs privileged `/dev/fuse`; SQLite page fetches are serial —
  VFS can batch/prefetch.

## Turso Cloud diskless (2025-04-07)

Source: <https://turso.tech/blog/turso-cloud-goes-diskless> — Glauber Costa

Measured from EC2 us-east-1, 1000 ops (gist linked in post):

| Op | S3 Standard avg / p95 / p99 (ms) | S3 Express same-AZ avg / p95 / p99 |
|----|----------------------------------|-------------------------------------|
| GET 4k | 19 / 23 / 42 | 3.8 / 4 / 4 |
| PUT 4k | 31 / 60 / 102 | 6.4 / 7 / 7 |
| PUT 128k | 53 / 124 / 199 | 5.5 / 7 / 7 |
| PUT 512k | 83 / 176 / 233 | 7.5 / 8 / 10 |

- Author compares Express +4 ms vs ~2 ms `fsync`.
- S3 Express: single AZ, durability SLA 11 nines, **uptime 99.95%** (one zone).
- Pricing cited: Standard PUT $0.005/1k, Express $0.0025/1k. 25M single-row
  TXs/mo → ~$125 Standard / ~$57 Express **per customer if 1 PUT/TX**.
- Fix: massive multi-tenant node; batch many DBs into one PUT; wait extra ms
  to accumulate. 100 active DBs → ~$0.57/DB-mo on Express PUTs.
- Local disk = **write-through cache**, not source of truth. Checkpoint folds
  WAL into main file every couple of minutes, then copy to Standard S3
  (continuous backup = keep old versions).
- Motivation: BYOC without Kubernetes StatefulSets.

## AgentFS → object (2026-01-11)

Source: <https://penberg.org/blog/disaggregated-agentfs.html> — Pekka Enberg

- AgentFS: POSIX-like FS + KV + toolcall audit, all in one SQLite
  (inode/dentry/whiteout). FUSE/NFS for real `git`/`grep`.
- Local single-file hits a wall: ephemeral compute, migrate, multi-agent.
- Direction: SQLite WAL = in-flight mutations; Turso sync = push logical
  mutations, pull physical pages; object store = source of truth.
- Extra: time-travel/branch from WAL; last-push-wins or transform hooks for
  multi-agent; offline then merge.
- Open (their words): write concurrency (SQLite single-writer; Turso MVCC
  not yet on in AgentFS); checkpoint latency; page write amp (1 B → 4 KiB);
  large files should live in S3 with metadata in SQLite.

## Willow / mvSQLite (2026-05-12 + README)

Sources: <https://su3.io/posts/willow> ; <https://github.com/losfair/mvsqlite>

- mvSQLite: SQLite VFS on FoundationDB. Full SQLite features. Time travel.
  Lock-free OCC (BEGIN CONCURRENT-like). Inherits FDB correctness/replication;
  **no 5 s txn limit**; SQLite txn ~39× larger than native FDB.
- Willow: agent harness. Agent FS as SQLite table, **large blobs offloaded
  to S3**. Version = 8-byte FDB read version + 2-byte batch id.
- 1,000 agents / OS process, each own SQLite on mvSQLite. Clients stateless,
  open/close in ms, unused DB costs only storage.
- Contrast: “synced local file” is right for one big DB + many readers;
  wrong for a million small DBs any node may claim next.

## Litestream VFS (2025-12-11)

Source: <https://fly.io/blog/litestream-vfs/> — Ben Johnson

- LTX = ordered page sets (from LiteFS), not raw page stream.
- Compaction: read backwards, keep latest copy of each page.
- Levels: L0 every 1 s (short retention) → L1 30 s → … → 1 h → daily snapshot.
- Trailer ~1% of LTX = page index. VFS: page number → (object, offset, size)
  → S3 Range GET. LRU for hot inner pages.
- VFS is **read-only**. Writes still go through the Litestream daemon + local
  primary. Poll L0 → near-realtime replica without downloading the DB.
- `PRAGMA litestream_time = '5 minutes ago'` = PITR query.

## SlateDB RFC-0001 Manifest (accepted RFC; compaction not in *that* text)

Source: <https://slatedb.io/rfcs/0001-manifest/>

- WAL is **not** per-`put`: batch by `flush_interval` / `flush_bytes` /
  `skip_memtable_bytes`; WAL objects are themselves SSTs.
- Fencing: bump `writer_epoch` in manifest (CAS), write empty fencing SST
  into next WAL slot; lower epoch seeing higher epoch halts.
- Manifest size calc in RFC: 100k compacted SSTs + 1k snapshots ≈ **5,628,042
  bytes (~5.6 MiB)**.
- S3-without-CAS path: two-phase write + transactional store (DynamoDB).
  RFC predates / does not assume S3 conditional writes (AWS 2024-08).
- Target latency in goals: < 100–300 ms read/write.
- Single writer, many readers, one compactor; may run on separate machines.

## turbopuffer architecture (live docs, fetched 2026-08-12)

Source: <https://turbopuffer.com/docs/architecture>

- Cold query 1M docs: **p50 = 874 ms**. Cached NVMe: **p50 = 14 ms**.
- Consistent read still hits object metadata: **~10 ms floor**.
- Eventual mode: lower latency, staleness **up to ~1 hour** worst case.
- Write: durable when WAL object exists. **p50 = 165 ms** for 500 kB.
  **1 WAL entry per second** per namespace; concurrent writes batch.
- ~3–4 object round-trips on cold path; they state ~100 ms per RT as
  first-principles.

## Tigris architecture (official)

Source: <https://www.tigrisdata.com/docs/concepts/architecture/>

- S3 API gateway (stateless) + global cache + distributor.
- **Metadata** (object location, user metadata, buckets, IAM): **FoundationDB**,
  multi-cluster replicated (QuiCK-style queue).
- **Object bytes**: **block stores**, placed near the user — *not* inside FDB.
- Offers CAS / multi-object TX / rich query **because metadata is FDB**,
  not because S3 grew transactions.

This contradicts the 2026-08-11 in-repo claim that Tigris is “FDB wearing
an S3 costume” as the *data* plane. Costume is the API; data plane is block.

## JuiceFS (docs)

Sources: <https://juicefs.com/docs/cloud/introduction/architecture/> ;
<https://juicefs.com/docs/community/databases_for_metadata>

- Split: metadata engine (Redis, TiKV, MySQL, PostgreSQL, SQLite, **FDB**
  since 1.1; Enterprise = custom) + object storage for bytes.
- File → chunks → slices → **blocks (default max 4 MiB)** for parallel PUT.
- KV metadata ~300 B/file; SQL metadata ~600 B/file. Large-scale rec:
  TiKV or FDB (community commentary, including TiKV co-founder on HN).

## S3 Express One Zone (AWS)

Source: <https://aws.amazon.com/s3/storage-classes/express-one-zone/>

- Single-digit ms, up to 10× vs Standard, request cost up to 50–80% lower
  (marketing pages disagree 50 vs 80 — treat as “cheaper PUTs”, use the
  Turso table for dollars).
- Directory buckets: up to **2M GET/s**, 200k write/s (AWS userguide
  performance page).
- Single AZ. Not a drop-in for “S3 but faster everywhere.”

## Cloudflare Durable Objects + SQLite (2024-09-26)

Source: <https://blog.cloudflare.com/sqlite-in-durable-objects/>

- SQLite as a library in the **same thread** as the DO. Storage latency
  ~zero with cache; durability is off-machine replication + later object
  stream (Rivet’s comparison; DO post itself emphasizes colocated thread).
- Architecture opposite of “stateless compute + remote pages.”

## sqlite-s3vfs / Turbolite (landscape only)

- simonw/sqlite-s3vfs: pages → fixed S3 blocks; **no locking**; archived
  2025-12-30. Writer must serialize externally.
- Turbolite (HN ~2026-04): Rust VFS, sub-250 ms cold JOIN from S3. Author:
  “experimental, buggy, and may corrupt data.”
