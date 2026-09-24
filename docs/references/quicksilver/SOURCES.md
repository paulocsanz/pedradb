# Quicksilver primary sources (fetched 2026-08-14)

Not a summary of conclusions — that lives in
[`../../slipstream-and-quicksilver-learnings.md`](../../slipstream-and-quicksilver-learnings.md).
This file is the source catalog + quotes pinned at fetch time.

## Primary posts (read in full)

| # | Date | URL | What it is |
|---|------|-----|------------|
| Q0 | 2020-03-30 | https://blog.cloudflare.com/introducing-quicksilver-configuration-distribution-at-internet-scale/ | Why KT failed; QS v1 design (LMDB + monotonic log + tree) |
| Q1 | 2020-11-25 | https://blog.cloudflare.com/moving-quicksilver-into-production/ | Dual-run migration; I/O; Raft root; first “everything everywhere is obsolete” |
| Q2 | 2025-07-10 | https://blog.cloudflare.com/quicksilver-v2-evolution-of-a-globally-distributed-key-value-store-part-1/ | v1.5 proxy/replica; MVCC; sliding window; negative lookups; discovery |
| Q3 | 2025-07-17 | https://blog.cloudflare.com/quicksilver-v2-evolution-of-a-globally-distributed-key-value-store-part-2-of-2/ | v2 tiered cache; relay; reactive prefetch; sharded L2 cache |

Secondary (not used as sole evidence):

- InfoQ 2025-08-09: https://www.infoq.com/news/2025/08/cloudflare-key-value-store/ (restates Q2+Q3)
- Workers KV (different product, eventual consistency, object-store backends):
  https://blog.cloudflare.com/building-with-workers-kv/ ,
  https://blog.cloudflare.com/faster-workers-kv/ ,
  https://blog.cloudflare.com/rearchitecting-workers-kv-for-redundancy/
- Internal-use note: https://blog.cloudflare.com/building-cloudflare-on-cloudflare/

Quicksilver is **not** open source (2020 post promised it; it did not land).
All architecture claims below are from the blogs, not from code.

## Scale numbers (as of Q3, July 2025)

- 330 cities / 125+ countries
- 5B+ KV pairs, 1.6 TB combined (dataset +50% in the prior year)
- 3B+ keys/sec worldwide
- p90 < 1 ms, p999 < 7 ms
- ~10 independent *instances* (separate DBs) per server
- Negative lookups ~10× positive lookups
- Pure keys ≈ 1/11 of full dataset size
- MVCC history window: ~2 hours, ~500 MB extra
- Working set (v1.5 study): ~20% of keyspace in large DCs, ~1% in small DCs
- v1.5 target cache fraction (3-day access) of 20% was “wildly off” for some instances
- v2 L1 hit rate ≥ 99.9% (worst instance, after reactive prefetch)
- v2 L1+L2 hit rate ≥ 99.99% worst instance, ≥ 99.999% others

## Q0 — KT → Quicksilver v1 (pinned)

KT problems they measured, not folklore:

- Exclusive-ish lock: flush-to-disk blocked reads. Same 20×2-byte keys:
  idle p99 9 ms / p999 15 ms; one 40 kB writer → p99 154 / p999 250 ms;
  two writers → p99 701 / p999 1215 ms.
- Disabled per-write fsync → corruption; repair too slow; SIGKILL on long
  shutdown → more corruption.
- Replication by *timestamp*, not sequence: missed GC’d log entries silently;
  `write_rts` only on loop exit → crash replayed days of already-applied logs.
- Dual-main required a single active writer (no auto failover).
- One process per DB file → no zero-downtime upgrade.

v1 answers:

- LMDB (later RocksDB): concurrent readers, crash-proof COW, snapshots of a live DB.
- Monotonic sequence in the replication protocol (`SET`/`DEL` with index).
- Incremental hash in the log; DB unique IDs in handshake; process UUID to
  refuse self-replication.
- 500 ms write batching.
- Log stored in the same LMDB env as data (one commit) → fragmentation;
  they later page-chunked the log.
- Per-KV CRC.
- Tree: root → intermediate → leaf. Secondary mains keep ~1 week of history.
- Heartbeat at the top of the tree to measure lag.
- Do **not** use Quicksilver-backed DNS to discover Quicksilver.

## Q1 — production + first obsolescence (pinned)

- Dual-run via QSKTBridge (KT feed → QS root, 500 ms batches, CAS on timestamp).
- Protocol compatibility (memcached + KT HTTP) so clients flipped via loopback.
- Bootstrap by LMDB copy-to-fd; parallel bootstrap thrashed page cache —
  they serialized it with a process lock.
- Most “replication delay” was **disk**, not network. Stale >30 s → reconnect
  to next source.
- Intermediate that stops serving while it hunts a new parent cascades
  disconnects (removed).
- Write amp on LMDB: 1 MB logical → ~30 MB disk; observed 1.5×–80×.
- Identical-KV writes dropped at the root.
- QSusage (bytes per producer) + QSanalytics (30-day access in ClickHouse,
  no sampling).
- RocksDB trial: ~40% of LMDB space, higher CPU (150% vs 70%), lower write amp.
- “A piece of data one megabyte in size consumes at least 10 gigabytes globally.”
- They started a sharded design; v2 later chose **cache**, not full-dataset shards.

## Q2 — v1.5 (pinned)

Topology: few core **root** nodes (TB disks); per edge DC **intermediate** +
**leaf**. Roles: **replica** (full dataset) vs **proxy** (persistent RocksDB cache).
First cut: 5 replica instances + 5 proxy instances per box (~50% disk).

Consistency (Hyrum): sequential — if A was written before B you cannot read B
and not A. Async replication + proxy miss path can violate this two ways:

1. Replica *ahead* of proxy → serve a future version. Fix: **MVCC** column
   family, lookup at the proxy’s replication index; tombstones for delete-then-
   recreate; GC via compaction filter (~2 h).
2. Replica *behind* proxy → “not found” for a key the proxy already applied.
   Rejecting the request was too common in the field. Fix: **sliding window**
   of recent updates on the proxy (not evictable until they age out).

Negative lookups: caching them failed (negative keyspace ~1000× live for some
instances; lost Bloom-speed). Cuckoo at 5B keys ≈ 18 GB RAM vs Bloom 6 GB.
Shipped: **keep all keys on the proxy, evict only values**.

Discovery: local Consul/DNS for in-DC replicas; **Network Oracle** (gossip +
RTT on intermediates) for nearby DCs. Connection pool + health isolation.

Eviction: compaction filter + in-memory LRU metadata; **soft** limit evicts;
**hard** limit stops admitting new cache keys (latency hit, stability preserved).

## Q3 — v2 (pinned)

v1.5 failed to scale because:

- Instance sizes drift (teams own the keyspace).
- 3-day working-set estimate was wrong for some instances (dnsv2).
- Cutting 40% disk is not a growth curve; 2 years later they were full again.

They considered sharding the **full** dataset and rejected it: complexity,
poor locality (1/N local hit), still stores cold keys.

v2: handful of **storage nodes** hold the full dataset. Everyone else is cache.
**Relays** (few elected per instance per DC) own the connections to storage
so every proxy does not fan in.

**Reactive prefetch:** every resolved miss on a relay is published; all
proxies in the DC subscribe and insert. Predictive prefetch was tried and
**abandoned** (no improvement).

**L2 sharded cache** (for dnsv2 tail latency): 1024 **logical** shards by
hash range → **physical** shards by range; hostname assigns one physical
shard per server. Doubling physical shards assigns each server a **subset**
of its previous range (no copy). Shard retention is longer than L1; L1 stays
for locality.

Cache shards do **not** do MVCC. If the shard’s write index is newer than the
proxy’s DB version, fall back to a storage replica. Rare because versions
stay close.

Three levels:

1. L1 — per-server recently-accessed cache
2. L2 — DC-wide sharded “accessed sometime” cache
3. L3 — full replicas on storage nodes (cold keys only)

Migration doctrine they state: iterative, revertible, transparent to clients.
v1 → v1.5 (learn distributed query) → v2.

## Workers KV is not Quicksilver

Workers KV is a **customer** product: eventually consistent, object-store
backends, tiered HTTP cache, 60 s consistency target. Quicksilver is the
**internal** sequentially-consistent config fabric on the request path.
Do not mix their consistency or topology lessons.
