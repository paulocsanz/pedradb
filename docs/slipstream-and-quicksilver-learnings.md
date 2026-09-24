# Slipstream + Quicksilver v2 — what they actually are, and what Pedra/Montanha should take

**Status:** research note (not an RFC, not a product commitment)
**Written:** 2026-08-14
**Primaries (read in full, persisted):**
[`references/quicksilver/SOURCES.md`](references/quicksilver/SOURCES.md),
[`references/slipstream/SOURCES.md`](references/slipstream/SOURCES.md),
[`references/slipstream/ARCHITECTURE.md`](references/slipstream/ARCHITECTURE.md)

This is not “two KV stores we should copy.” They solve **two different jobs**
that Pedra’s north star currently smears together. The useful move is to
name those jobs, keep Montanha on the transactional one, and steal a small
set of *invariants* for the other.

---

## 0. One-screen conclusion

| | **Quicksilver** (Cloudflare, internal) | **Slipstream** (beyondoss, MIT, 0.7.2) | **Pedra + Montanha today** |
|--|----------------------------------------|----------------------------------------|----------------------------|
| Job | Sequentially-consistent **config fabric** on the request path | **Fold** a bounded JetStream KV into a local replica that survives restart and log eviction | Local ACID KV (Pedra) + lab multi-Raft SoR (Montanha) |
| Write path | Single logical root (now Raft cluster); 500 ms batches | NATS CAS / put; last-write-wins per key | Pedra TX + Montanha Raft leader |
| Read path | Local disk, µs–ms; 3B keys/s | Local fold `get`/`range`; NATS only for catch-up | Local Pedra; Montanha TCP to leader |
| Replica shape | v1: **everything everywhere**. v2: **cache everywhere, full data on a handful of nodes** | Every consumer is a fold. After NATS eviction, **folds are the only full replicas** | WAL-ship replica; `WalRotated` ⇒ re-bootstrap by hand |
| Consistency | Sequential (Hyrum-locked). Async tree + MVCC + sliding window | Cursor-after-apply. Not multi-key TX | Local SI/OCC; Raft majority for cluster writes |
| The load-bearing idea | Do **not** shard the full dataset first. Cache + measure working set. Preserve seq consistency across async hops | Do **not** advance a cursor on *receipt*. Treat silent log clamps as **data loss** | CHANGELOG is already a cache; `JournalConsumer` still advances the pin on *read* |

**Do not** turn Montanha into Quicksilver. Quicksilver is a **push-replicated
read-mostly config log**, not a multi-key ACID store. Workers KV (eventual,
object-store backends) is a third thing — ignore it for SoR design.

**Do** steal, in this order:

1. Cursor-after-apply for every feed/watch/ship consumer (Slipstream).
2. Fail-closed resume when the log has forgotten you (Slipstream `resume_window_ok` + QS “reconnect if 30 s stale”).
3. Snapshot + cursor bootstrap, not “replay the WAL forever” (both).
4. Split **config-distribution** from **transactional SoR** before we put a
   full Montanha replica on every edge box (Quicksilver’s 10-year lesson).
5. If we ever cache at the edge: persistent cache, keep-keys/evict-values,
   reactive prefetch, sharded *cache* not sharded *dataset*.

---

## 1. They are not the same product

Pedra’s platform sentence (“replace Scylla + ClickHouse + NATS need”) names
three *needs*. These two systems cover two of them, and they do not share a
consistency model.

```
                    writes rare, reads everywhere, sequential
                    ┌─────────────────────────────────────┐
   control plane ──►│  Quicksilver-class config fabric     │
                    │  (async log, tree, edge cache)       │
                    └─────────────────────────────────────┘

                    last-write-wins bucket, bounded log
                    ┌─────────────────────────────────────┐
   NATS KV / routes─►│  Slipstream-class fold               │
                    │  (watch + local SnapshotStore)       │
                    └─────────────────────────────────────┘

                    multi-key ACID, any-node-accept later
                    ┌─────────────────────────────────────┐
   app TX ─────────►│  Montanha / FDB-class SoR            │
                    │  (Raft/TSO/OCC — what we are)        │
                    └─────────────────────────────────────┘
```

Slipstream is the honest replacement for **“NATS JetStream KV + every edge
process rebuilds a HashMap.”** It is the consumer of a log, not the log.

Quicksilver is the honest replacement for **“Scylla (or KT) as the thing
every FL/DNS/WAF process hits on every request.”** It is a purpose-built
replicated config DB with a single writer and a planet-scale read fanout.

Montanha is the honest replacement for **FoundationDB / TiKV as SoR.**
Mixing (2) or (3) into (1)’s topology — full dataset on every leaf, or
proxy-cache in front of a linearizable commit path — is how you inherit
the wrong failure mode.

We already wrote the NATS half of this in
[`nats-need-replacement.md`](nats-need-replacement.md):
Core NATS ≠ JetStream; ack-before-fsync is a Jepsen finding; Pedra replaces
the *durable stream job*, not the wire protocol. Slipstream is what you
build **on top of** a stream once you admit the stream is bounded.

---

## 2. Quicksilver — detailed

Sources: Q0 (2020 intro), Q1 (2020 production), Q2 (2025 v1.5), Q3 (2025 v2).
Numbers and mechanisms below are from those posts, not from code (QS is
closed). See [`references/quicksilver/SOURCES.md`](references/quicksilver/SOURCES.md).

### 2.1 The job

Every Cloudflare request path (DNS, TLS certs, WAF, Workers routing, CDN
config) needs a few to a few hundred keys, in <1 ms p90, even if the DC is
cut off from the core. Writes are relatively rare (tens of millions/day in
2020; still tiny vs 3B keys/s reads in 2025). The API is get/scan-shaped,
not multi-key ACID.

That is **not** Montanha’s job. It is closer to “Pedra as an embedded
read replica of a config log,” which we do not have as a product.

### 2.2 Evolution (the part we will otherwise skip)

| Gen | Shape | Why it died |
|-----|--------|-------------|
| KT | Kyoto Tycoon, timestamp replication, exclusive flush lock | Read tail exploded under writes (p999 15 ms → 1.2 s). Silent missed logs. Corruption after they killed fsync. 48 SRE-hours/week. |
| **v1** | LMDB (later RocksDB) on **every server**, monotonic log, static then gossip tree | 1 MB of config = ≥10 GB fleet-wide. Disk + SSD wear. Bootstrap page-cache thrash. |
| **v1.5** | Per-instance **replica vs proxy** (persistent cache). Even split: 5+5 instances/box | Instance sizes drift. 20% working-set guess was wrong. −40% disk is not a growth curve (full again in ~2 years). |
| **v2** | Handful of storage replicas. Relays. L1 local cache + L2 DC-sharded cache. Reactive prefetch | Current. Worst-instance in-DC hit ≥ 99.99%. |

Two process lessons they state explicitly:

1. **Ship an intermediate that is revertible** (v1.5) so you learn distributed
   query + MVCC + discovery *before* you bet the fleet on tiered cache.
2. **Hyrum’s law.** Sequential consistency was an accident of v1 (every node
   had the same log prefix). Clients now depend on it. v1.5/v2 spend most of
   their complexity *preserving* that, not on caching.

### 2.3 v1 mechanics worth keeping on a whiteboard

- **Monotonic index, not timestamps.** KT’s timestamp protocol missed GC’d
  entries with no error. QS’s log is `N SET k v` / `N DEL k`. A gap is
  detectable. Pedra already has `SequenceNumber`; WAL-ship uses a *byte*
  cursor, which is the KT-shaped mistake (see §5.2).
- **Batch the log (500 ms)** so durable writes do not serialize behind
  per-update fsync. We have group commit. A *distribution* batch is still
  missing.
- **Log + data in one commit** (they put the txlog in LMDB). Then they paid
  fragmentation and had to page-chunk the log. Pedra’s WAL-as-SoR + CHANGELOG
  as cache is the better split — do not put the ship log inside SST.
- **Incremental hash** in the log (detect reorder/drop). We have per-record
  CRC; we do not have a running hash across the shipped prefix.
- **Heartbeat at the root** to measure replication lag. We have
  `/v1/cluster` membership, not a lag probe on the apply index.
- **Do not discover yourself via a name that lives in you.** QS refused to
  use QS-backed DNS for QS topology. Our PEERS-via-VIP problem
  ([`proxy-raft-replicas-proposal.md`](proxy-raft-replicas-proposal.md)) is
  the same class.
- **Identical put drop** at the root. Cheap SSD win.
- **QSusage / QSanalytics.** Teams could not see bytes or unused keys.
  Cold keys are *the* reason v2 exists. We have no per-prefix usage.

LMDB → RocksDB (Q1): space ~40%, write amp down, CPU up, online compact.
Pedra’s LSM+vlog path is already on the RocksDB side of this argument;
[`compare-fjall.md`](compare-fjall.md) is the local analog.

### 2.4 v1.5 — the consistency tricks

Async replication + a miss path to another node **breaks sequential
consistency** in two directions. They drew it:

```
client writes A,B,C,…K at the root
proxy            @ index 5   (has A..E)
replica_1        @ index 2   (has A,B)     ← behind
replica_2        @ index 9   (has A..I)    ← ahead
```

A proxy that misses F..I, load-balances, can see “all present” then “all
absent.” Both answers are wrong relative to v1 (which would have returned
only E).

| Case | Fix | Pedra analog |
|------|-----|----------------|
| Replica **ahead** | **MVCC** CF: store old versions keyed by index; serve `get(k) @ proxy_index`. Tombstones for delete-then-put. GC via compaction filter (~2 h, ~500 MB). | We have `get_at` / SI snapshots. We do **not** expose “serve this read at replica index N” on the Montanha wire. |
| Replica **behind** | First they returned an error. Field said no. **Sliding window** of recent updates on the proxy: not evictable, restored from index + window size after crash. | A proxy that applied seq 100 must not ask a replica at seq 80 for a key it just wrote. |
| Negative lookup | 10× of traffic. Caching absences: negative keyspace ~1000×. Cuckoo at 5B keys = 18 GB. **Keep all keys, evict values.** Bloom stays fast. | We have vlog + blooms + `ScanProjection::KeyOnly`. A cache-role node is “keys in LSM, values optional in vlog.” |
| Eviction | Compaction filter + in-RAM LRU dates. Soft disk limit evicts; **hard** limit refuses new cache inserts (hurts hit rate, saves the box). | No compaction-filter hook. No cache-role. |
| Discovery | Consul in-DC; **Network Oracle** (gossip + RTT on intermediates) for nearby DCs. Pool + isolate bad replicas. | Hardcoded PEERS / RoleAware proxy. No RTT oracle. |

They almost sharded the full dataset here and **did not**, because a 4-way
shard gives 25% local hits and still stores cold keys.

### 2.5 v2 — tiered cache

New fact: most keys are **cold** (stale, regional, never-read domains/scripts).

```
request
  │
  ├─ L1 local cache (this server, recently used)
  │     miss
  ├─ L2 DC sharded cache (this server holds one physical shard of “used sometime”)
  │     miss, or L2 version newer than this proxy’s index → ignore
  ├─ relay (few per instance per DC)
  │     │
  └─────┴─ L3 storage replica (full dataset, MVCC window)
              resolved miss published to all proxies in the DC  ← reactive prefetch
```

**Relay** exists so a handful of storage nodes are not crushed by
`servers × instances` connections.

**Reactive prefetch** (miss stream from relays → every proxy) took the worst
instance to ~99.9% L1. **Predictive** prefetch (guess unread keys) did
nothing; they killed it. Steal the humility: measure, don’t invent a
prefetcher.

**L2 sharding** is cache sharding:

- hash(key) → 1024 logical shards (range)
- logical ranges → physical shards (range)
- `hash(hostname)` → one physical shard
- **Double physical shards ⇒ each server keeps a subset of its old range.**
  No copy. Extra keys evict over time.
- L2 does **not** implement MVCC. If `key.write_index > proxy.db_index`,
  skip L2 and go to L3. Rare because indexes stay close.

This is the opposite of Montanha range split (which shards the *source of
truth*). Do not reuse `split_range_at` as if it were an edge cache.

### 2.6 What Quicksilver is *not*

- Not multi-writer per key.
- Not strictly serializable TX.
- Not a good model for Montanha commit path (we already chose multi-Raft
  ranges: [`rfc/0021-commit-path-scale-decision.md`](rfc/0021-commit-path-scale-decision.md)).
- Not Workers KV (eventual, object store, 60 s).
- Not open source. We cannot copy RocksDB CF layouts; we can copy *roles*.

---

## 3. Slipstream — detailed

Sources: repo README, ARCHITECTURE.md (71 kB, read in full), BACKENDS.md,
`src/protocol.rs`, Cargo.toml 0.7.2. Persisted under
[`references/slipstream/`](references/slipstream/).

### 3.1 The job

```
NATS JetStream KV  (bounded: max_bytes / max_age)
        │  KvUpdate stream (Put / Delete / Purge)
        ▼
  watch_applied()     parse → apply(batch) → THEN cursor = batch_high
        │
        ▼
  SnapshotStore       fold is a pure function of the log
        │
   restart: resume from cursor          cursor expired: export/import
   (delta only)                         (folds are the only full replicas)
```

Config (routes, certs, WASM) lives in a **bounded** log. A consumer that
only has a sequence number cannot rebuild after eviction. So:

- the local fold **is** the durable replica
- a content-addressed artifact in object storage is how a *new* node is born
- NATS is SoR only while your cursor is still inside the retention window

This is the missing half of [`nats-need-replacement.md`](nats-need-replacement.md)
§3.3 (hot bus + cold durable log). Slipstream is the **edge materializer**.

### 3.2 Layering (steal the trait cut)

```
KvReader / KvWatcher / KvWriter / KvTtl     object-safe, optional writer/watcher
        │
     KvStore                                named bucket
        │
    Connection                              connect / health / store factory
        │
   NatsConnection                           one backend today; VersionToken also fits FDB
                                            (8-byte u64 or 10-byte versionstamp)

orthogonal:
   SnapshotStore { load, apply(batch, cursor), get, range }
     AppendLogSnapshot | FjallSnapshot | RocksDbSnapshot

combinator:
   watch_applied(watcher, store, parse, apply, on_applied)
```

Serving structures (hashrings, route tables) live **in the consumer**,
queried out of the fold. The store stops at fold + cursor + query. That is
the same cut as “Pedra is the local primitive; the product is a layer”
([`architecture-refined.md`](architecture-refined.md)).

### 3.3 The combinator we keep getting wrong

Every hand-rolled caller advanced the cursor at `rx.recv()`. Crash window:

```
recv rev N  →  persist cursor=N  →  CRASH  →  apply never ran
resume at N+1  →  silent hole
```

`watch_applied` makes the invariant un-missable:

> persisted cursor `C` ⇒ `apply()` has returned for every revision ≤ `C`.

Flush order: `apply(batch)` → `cursor = batch_high` → `store.apply(raw, cursor)`
on a blocking task → `on_applied`. Batch closes on 10 ms or 100 updates
(defaults). `parse → None` still advances the cursor (nothing to apply ≠
something to skip). Unknown-version ACKs never touch `batch_high`.

**Transient `store.apply` error:** prepend the raw batch, increment a streak,
do **not** move the cursor. 16 failures → fail-stop. Dropping the batch and
continuing was a shipped bug; the model test
`transient_store_failure_never_leaves_a_cursor_gap` is the proof.

This is **directly** our `JournalConsumer::catch_up`:

```rust
// crates/pedradb-journal/src/lib_kernel.rs
let batch = db.changes_after(self.pin);
// ...
if let Some(m) = batch.iter().map(|e| e.sequence).max() {
    self.pin = m;   // advanced on *read*, not on consumer apply
}
```

In-process canaries are fine. The moment a layer persists `pin` and then
builds a route table / hashring / secondary index, this is the Slipstream
footgun. RFC-0019 gave us the seam; it did not give us the combinator.

### 3.4 Cursor expiry is a delete-marker problem

NATS `watch_from(cursor)` on a compacted stream **does not error**. It
clamps to `first_sequence` and you skip evicted **delete** markers. Live
keys come back on a full re-list; keys that vanished in the gap stay
forever. They pinned this with
`tests/resync.rs::nats_silently_clamps_resume_below_first_seq`.

Guard (same function in prod and in Stateright):

```text
resume_window_ok(revision, first_sequence)
  ⇔  first_sequence ≤ revision + 1
```

On fail: list live keys (headers only) → synthetic `Delete` for fold keys
not in the list (unknown version, **does not** advance cursor) → ack →
*then* `watch_all` re-list. Ordering is the invariant: a recreate in the
gap must not see put-before-synthetic-delete.

**Live** overrun (All-scope): in-band, a delivery that jumps the frontier
by >1 is checked against `first_sequence` *before* the fold sees it. 30 s
probe covers the no-traffic case. Periodic-only was rejected by the model
(a delivery can erase the evidence between probes). **Prefix** watches
cannot use this (gaps from non-matching subjects look identical). Their
operating axiom: retention must outlive the restart interval.

Pedra CHANGELOG is a cache rebuilt from WAL, so *open* cannot silently
clamp — good. A **networked** watch on Montanha can, the moment we compact
the feed or ship a truncated WAL. `ShipError::WalRotated` is exactly
`CursorExpired` without a resync protocol.

### 3.5 Snapshot format + backends

Append log (default, RAM fold):

```
PGSS ++ version:u16le
record = crc32 ++ type ++ payload
  PUT    key + value + version_bytes (≤10, not a fixed u64)
  DELETE key + version_bytes
  CURSOR cursor_bytes
```

Truncated tail discarded. Mid-file CRC → `Corrupted` (delete, full replay).
`checkpoint` flushes to page cache; **only `compact` fsyncs**. They accept
power-loss tail loss because the watch re-folds. That is *their* durability
story (NATS still has the tail). It is **not** Pedra’s commit story
(RFC-0019: WAL sync before Ok; CHANGELOG must not gate commit). Do not
copy “checkpoint without fsync” onto `Db::commit`.

LSM backends: one atomic batch = data **and** cursor. `sync: false` default
(NO_SYNC). After bulk hydrate, **`settle()`** or cold gets are 8–10× slower.
fjall hydrates faster, settles in ~19 min at 500M with 2× disk; RocksDB
hydrates slower, settles in ~40 s, better p999 (898 µs vs 3.7 ms). Pick
with numbers, not brand.

### 3.6 Bootstrap when the log has forgotten you

```
ExportLease::try_acquire (CAS create; steal expired/corrupt)
  → flush pending batch
  → store.export_to (verify-by-reopen: recovered cursor == live cursor)
  → tar payload at blake3(manifest)[..8]
  → CAS swap pointer if pointer_publish_allowed
  → complete(cursor) / prune (strictly-below + 4×TTL grace)
  → delete local artifact (must not pin compacted SST hardlinks)
```

Three kernels, **one copy**, called from production and from
`tests/model.rs`:

| Kernel | Theorem |
|--------|---------|
| `pointer_publish_allowed` | published cursor never regresses |
| `payload_prunable` | pointer target always fetchable (zero-grace) |
| `resume_window_ok` | bootstrap never silently diverges |

Age-only prune produced a dangling pointer in the model; strictly-below
is the structural fix. `file://` object stores lack CAS and **fail closed**
unless a dev-only flag is set.

This is the object-store rung we already called “plausible above the
kernel” in [`object-storage-as-substrate-possibility.md`](object-storage-as-substrate-possibility.md).
Slipstream is a concrete protocol for it: **artifact of a fold**, not
“database on S3.”

### 3.7 Other sharp edges (short)

- `scan`/`keys`: ephemeral push consumer, `LastPerSubject`, `AckPolicy::None`
  (Explicit silently stops at `max_ack_pending=1000`). Subscribe **before**
  create (async-nats ≤0.46 race). ACK subject parser is version-sensitive
  (9 vs 11–12 tokens); they used to take the last token (`num_pending`) and
  mint a wrong version on every scan.
- Every NATS op has a 30 s timeout (CLOSE_WAIT otherwise parks forever).
- State-sync watches: no scan-then-watch race. Documented in
  `watch_prefix_relist_covers_seed_then_watch_gap`.
- Capabilities flags (`cas`, `streaming_watch`, `prefix_watch`,
  `global_ordering`) — FDB versionstamps are **not** globally ordered
  across keys; NATS revisions are per-key. `VersionToken::as_u64()` is
  `None` for FDB.
- Corrupt lease/pointer is replaceable. Unknown state is not a wedge.

---

## 4. Other sources (what they add)

| Source | Adds | Does not add |
|--------|------|----------------|
| InfoQ 2025-08-09 | Confirmation of v2 numbers; no extra mechanism | Independent evidence |
| Q0/Q1 KT post-mortem | Timestamp replication and ack-before-durable are the same *shape* as Jepsen JetStream | A storage engine to use |
| Workers KV blogs | A *different* CF product: eventual, multi-object-store, tiered HTTP cache | Sequential consistency, Raft, or Pedra’s commit path |
| Consul blocking queries (cited by Slipstream) | Watch from last **reconciled** index | Storage |
| Saltzer/Reed/Clark 1984 (cited by Slipstream) | Checkpoint below the endpoint is a hint | Implementation |
| Jepsen NATS 2.12.1 + our [`nats-need-replacement.md`](nats-need-replacement.md) | JetStream ack ≠ fsync; `.blk` / snapshot corruption | How to fold a stream |
| Pedra RFC-0019 | We already have CAS, seq pin, CHANGELOG, `multi_get`, KeyOnly | The consumer combinator and expiry protocol |
| Pedra `pedradb-replicate` | Byte-cursor WAL ship | Snapshot+cursor bootstrap; logical seq cursor |
| FDB / TiKV (our existing docs) | SoR distribution. Orthogonal to QS cache | Edge working-set design |

No Slipstream academic paper. No Quicksilver source. Claims about QS
internals stop at the blogs.

---

## 5. Mapping onto Pedra / Montanha (as the code is, 2026-08-14)

### 5.1 What we already got right

| Their idea | Where we already have it |
|------------|--------------------------|
| WAL is SoR; derived feed is a cache | `Db` commit path: CHANGELOG store failure is a warn; reopen rebuilds from WAL (RFC-0019, F33) |
| Seq pin on commit | `put_with` / `commit` / `apply_batch` return `SequenceNumber` |
| CAS | `compare_and_swap` / `put_if_*` |
| Key-only listing | `ScanProjection::KeyOnly` |
| Batch point reads | `multi_get` / `multi_get_at` |
| MVCC read-at-version | `get_at` / SI (RFC-0023) |
| CRC fail-closed on WAL | WAL reader; CHANGELOG corrupt → quarantine, not brick open |
| Checkpoint as bootstrap | `Db` checkpoint copies CHANGELOG (F46) |
| Group commit | Dual-mem + group commit |
| Vlog (values off the point-read path) | WiscKey-class; the QS “keep keys, drop values” cache is a *policy* on this |
| DST / sim | `pedradb-dst`, FailingEnv — same *intent* as Stateright on kernels |
| Role-aware proxy | `montanha-tcp proxy` write→leader, read→any |
| Do not VIP the Raft mesh | [`proxy-raft-replicas-proposal.md`](proxy-raft-replicas-proposal.md) |

### 5.2 Concrete holes (code-backed)

**H1. Journal pin advances on read.**
`JournalConsumer::catch_up` sets `pin = max(batch)` before the caller
applies. There is no `watch_applied`. Any future Montanha watch / route
fold / SQL CDC projector that persists that pin will skip on crash.

**H2. WAL-ship cursor is a byte offset; rotate is a cliff.**
`ShipError::WalRotated { file_len, cursor }` — “re-bootstrap replica
(copy SSTs/MANIFEST).” That is KT/QS-v1 bootstrap pain with none of the
QS “snapshot of a live DB + log tail” or Slipstream “artifact +
resume_window_ok” protocol. After flush, a replica cannot catch up from
a sequence number.

**H3. No networked watch.**
CHANGELOG is poll/`changes_after`. No state-sync stream, no
scan-then-watch protection, no floor guard. Fine for L1. Fatal the
moment we claim “replace Scylla push / NATS watch.”

**H4. Geo is a dial hint.**
RFC-0021 P2.6: `set_node_region` / prefer-region dial. Not async DR, not
a replica/proxy role, not a working-set cache. If we “put a Montanha
node in every region” we are building **Quicksilver v1** (everything
everywhere) on a Raft commit path — the most expensive possible mistake.

**H5. No cache role, no compaction filter, no hard disk limit.**
A leaf that cannot evict is a replica. QS spent five years learning that.

**H6. Protocol kernels are not extracted.**
DST tests re-state invariants in the test. Slipstream’s `protocol.rs` is
the better shape: prod and model call the **same** three functions;
mutation tests prove each is load-bearing. We should do this for
`resume_window` (ship), `pointer_publish` (if we grow object artifacts),
and “cursor may not advance past unapplied seq.”

**H7. No usage/access analytics.**
QS built ClickHouse of *every* key access (no sampling) before they
trusted a 20% cache. We are guessing if we size an edge cache today.

**H8. Bootstrap / export is a drill, not a protocol.**
`montanha_backup_restore_drill.sh` proves backup. It is not a
content-addressed, monotone-pointer, verify-by-reopen fold export.

### 5.3 Two products, one kernel

```
                    ┌─ pedra-stream / journal ─┐
                    │  durable log, ack=fsync   │   ← nats-need-replacement
App ─► Pedra L1 ────┤                           │
       (this repo)  │  pedra-fold (new)         │   ← Slipstream job
                    │  watch_applied + snapshot │
                    │                           │
                    │  Montanha                 │   ← FDB/TiKV job
                    │  multi-Raft SoR           │
                    └─ qs-class edge cache ─┐   │   ← Quicksilver job
                       (much later)         │   │
                                            ▼   ▼
                                    do not put SoR
                                    replicas on every leaf
```

Pedra stays the local primitive. The fold and the cache are **layers**.
The SoR is Montanha. Shipping “Montanha on every caixote host as a full
replica of everything” is v1 Quicksilver with Raft fsyncs.

---

## 6. What to change — ranked

Bands are delivery, not importance. P0 here means “do before we grow
watch/CDC or geo replicas,” not “stop RFC-0023.”

### P0 — close silent-wrong windows we already have seams for

| ID | Change | Where | Why (evidence) |
|----|--------|-------|----------------|
| L1 | **`watch_applied`-shaped combinator** for feed consumers: `apply(batch)` then persist pin. Pin on receipt is a hole. | `pedradb-journal` first; any Montanha watch later | Slipstream shipped the bug, then encoded the fix. Our `JournalConsumer` is the pre-fix loop. |
| L2 | **Logical cursor for WAL-ship**, or treat rotate as `CursorExpired` with a **required** checkpoint+import path. Stop telling operators to copy SST trees. | `pedradb-replicate` | QS: monotonic index, not timestamps/bytes. Slipstream: `resume_window_ok`. We have `WalRotated` and no resync. |
| L3 | **Sequence-gap fail-closed** on CHANGELOG vs `last_sequence` when serving a *networked* subscriber. Poll-in-process can rebuild from WAL; a remote pin cannot. | core feed + future watch | NATS silent clamp; our F33 “treat corrupt CHANGELOG as empty” is correct for *open*, wrong if we already told a peer “you are at seq N.” |
| L4 | Extract **pure kernels** (`resume_window_ok` analog, “pin ≤ applied”, later pointer guards) into a module DST and prod both call. Mutation test that a broken variant is caught. | `pedradb-dst` / small `protocol` module | Slipstream `protocol.rs` + Stateright. We already believe in this method (RFC-0018). |

### P1 — watch / CDC / replica bootstrap (the Slipstream product)

| ID | Change | Notes |
|----|--------|-------|
| L5 | State-sync watch: first delivery is `changes_after(0)` / last-per-key, **then** tail. Never document scan-then-watch. | `watch_prefix_relist_covers_seed-then-watch_gap` |
| L6 | Cursor-expired resync: key-only live list, synthetic deletes, *then* re-list. Synthetic delete does not advance pin. | Only needed once the log/feed is bounded or compacted |
| L7 | Transient apply re-queue; fail-stop after a streak. Never skip a failed fold batch. | Copy the 16-failure number only after we measure |
| L8 | Fold export: checkpoint + manifest cursor + hash + verify-by-reopen. Object store optional (`transport` feature analog). Pointer CAS + strictly-below prune if we share artifacts. | Aligns with existing checkpoint; do not invent a second snapshot format without a cursor field |
| L9 | `settle()` (or `compact_for_reads`) after bulk hydrate of a replica, **before** serving. We already have `compact_for_reads` (RFC-0019 P2.2). Make it part of the replica contract, not an optional nicety. | Slipstream 8–10× unsettled penalty |

This *is* the “replace NATS KV at the edge” layer. It is not Montanha TX.

### P2 — do not build Quicksilver v1 by accident (geo / edge)

RFC-0021 P2.6 is a dial order. Before anyone deploys “a full replica per
region / per host”:

| ID | Change | Notes |
|----|--------|-------|
| L10 | **Name the roles** in a design RFC: `storage` (full, MVCC window), `relay` (connection aggregation), `proxy` (persistent cache). Default new nodes to proxy. | QS v1.5 existed so they would not jump to v2 blind |
| L11 | **Measure working set** (access log → ClickHouse/CH-need layer, or a Pedra prefix) for one real keyspace before sizing a cache. No sampling if we can avoid it. | Their 3-day/20% estimate was “wildly off” |
| L12 | If we cache: **persistent** (Pedra, not RAM), **keep keys / evict values**, soft+hard disk limits, compaction-filter or equivalent LRU-in-compact. | Negative lookups were 10×; Cuckoo lost |
| L13 | Replica-ahead: serve `get_at(proxy_apply_index)` from storage (we have `get_at`). Replica-behind: sliding window of recent applies on the proxy; do not miss-fill from a lagging peer. | This is how they kept sequential consistency |
| L14 | **Reactive** miss-share inside a DC/AZ. Do not start a predictive prefetcher. | They tried predictive; it did nothing |
| L15 | Sharded **cache** (1024 logical, physical by range, double = subset) only after L1+prefetch miss the tail. Do not shard the SoR to save leaf disk. | Opposite of `split_range_at` |
| L16 | Relays before we attach N proxies to 3 storage nodes. | Connection fan-in, not CPU |
| L17 | Heartbeat / apply-index lag on `/v1/cluster`. Reconnect (or fence reads) if lag > bound. | QS: 30 s stale → next parent |
| L18 | Per-tenant / per-prefix usage (QSusage). Identical-put drop at the write root. | SSD and politics |

### Explicit non-goals (do not “learn” these)

- Full dataset on every Montanha node “for locality.”
- Sharding the transactional keyspace *in order to* free leaf disk.
- In-memory-only caches for billion-key folds.
- Predictive prefetch.
- Timestamp / byte-offset as the public resume token.
- Advancing a watch pin in the transport layer.
- Treating JetStream (or any bounded log) as SoR after eviction.
- Copying QS sequential-consistency *into* Montanha’s linearizable commit
  (we already have a stronger local model; don’t weaken it to match QS).
- Copying Slipstream’s “checkpoint without fsync” onto Pedra commit.
- Depending on Montanha-backed DNS/Service discovery to find Montanha.
- Changing watch/seq semantics after the first external caller (Hyrum).
  Lock the pin-after-apply rule **now**, while we are the only caller.

---

## 7. Suggested RFC cut (when we implement, not now)

If this turns into work, do **not** stuff it into RFC-0021. Two small RFCs:

1. **`watch_applied` + ship cursor** (P0/P1 above) — kernel/journal/replicate.
   Parent: RFC-0019. Acceptance: crash between recv and apply cannot skip;
   `WalRotated` has an automatic snapshot+resume path; DST mutation test
   on the kernel.
2. **Config-fold / edge-cache roles** (P2) — only after a measured working
   set and a named product (this is not Montanha SoR). Parent: north-star
   + `nats-need-replacement.md`. Acceptance: a proxy node’s disk is a
   function of working set, not of cluster data size.

Until then this note is the record.

---

## 8. Dead ends / limits of this reading

- **No Quicksilver source.** CF layouts, RPC, exact MVCC key encoding, and
  the distributed KV protocol are undescribed. We must not invent them.
- **Slipstream is NATS-shaped.** Prefix-watch floor-guard impossibility is
  a NATS delivery property. A Pedra watch we control can carry a dense
  sequence and keep the in-band guard for prefix scopes — we should, rather
  than copy their operating axiom.
- **Workers KV** was checked and set aside: different consistency, different
  backend. Using it as evidence for Montanha or for QS-class sequential
  config would be a category error.
- **We did not run their benches.** fjall vs RocksDB numbers are theirs
  (500M routes, their hardware). Our `compare-fjall.md` still stands for
  *Pedra vs fjall as a kernel*; Slipstream’s numbers are about *fold
  backends*, a different job.
- **Nothing here moves P0 of the kernel.** It confirms O1 (sync before Ok)
  and the “log is a first-class artifact” line. It argues against growing
  geo replicas without roles.

---

**Caixote (2026-08-14):** o mesmo desenho aplicado à orquestração de
VMs — e por que federation-api + PG **não** chega — está em
`../caixote/docs/research/quicksilver-slipstream-orchestration-scale.md`.

## 10. Montanha é o lugar errado para a escala Slipstream/QS?

**Não é o lugar errado para o SoR. É o lugar errado para o *hot path
de leitura*.** Não precisamos de “outra DB transacional em cima”.
Precisamos de um **fold** que usa Pedra local e *consome* o log do
Montanha.

### 10.1 Teto que é física, não implementação

| Caminho | Slipstream / QS | Montanha (contrato) | Lab que medimos (`findings/perf-gate-v0-verify`) |
|---------|-----------------|---------------------|--------------------------------------------------|
| Write | QS: root + 500 ms batch, async tree. Slipstream: put no JetStream | Raft majority + Pedra sync. RFC-0021: escala por **mais ranges**, não por skip de fsync | put p50 **246 ms** / 3.6 qps — *lab in-process*, não teto de campo |
| Read linearizável | QS não oferece. Slipstream `get` no NATS é raro | `get_strong` → só o leader | — |
| Read local | QS p90 < 1 ms, 3B keys/s (RocksDB no leaf). Slipstream fold p50 292 µs cold (Rocks) | `get` = `LocalApplied` no Pedra **desta** caixa (`lib.rs` get) | get p50 **0.07 ms**, 13k qps (200 gets, um processo) |
| Quem vota no consenso | Folhas **não** votam. QS v2: punhado de storage nodes | Membros do Raft (3/5) | 3 nós |

O número de 246 ms **não** prova que Montanha “não aguenta” writes de
CP (são raras). Prova que o lab ainda não é um raft de 1–5 ms LAN.
Mesmo otimizado, **uma escrita Raft nunca é um `get` local.** QS
ganha em escala porque **3 bilhões de keys/s não passam pelo root.**

### 10.2 O que o Montanha *não* pode ser, por contrato

1. **Todo `caixote-api` / procurador como voter.** Consenso com 10⁵
   membros não existe. QS v1 já morreu em disco; QS v2 tirou o dataset
   das folhas. Montanha com N hosts como peers é o mesmo erro.
2. **`get_strong` no caminho do request.** Linearizável = hop no
   leader. Slipstream/QS são “seq do *meu* log local”.
3. **Réplica full do SoR em toda folha.** 1 MB → 10 GB fleet (QS).
   Multi-Raft *shard* o SoR; não substitui cache de working set.
4. **Misturar papéis no mesmo processo sem nomear.** Raft+Pedra no
   mesmo binário (decisão A, RFC-0021) é certo para o *storage node*.
   É errado para o *proxy*.

Nada disso é “Montanha é lenta demais então joga fora.” É “o contrato
de majority-Ok não é o contrato de 3B reads/s.”

### 10.3 O que o Montanha *já* dá para o fold

- Log ordenado por range (`SequenceNumber` / índice Raft).
- `get` LocalApplied — o mesmo shape que o fold serve.
- `get_at` / SI — o MVCC que o QS inventou no proxy-replica.
- CHANGELOG + CAS (RFC-0019).
- `ReadPolicy::LocalApplied` vs `Strong` já separados.

Shipped (RFC-0024): `pedradb-fold` — `watch_applied`, pin-after-apply, prefixo
por host, bootstrap por artifact, evicção. Isso é camada, não
segundo Raft.

### 10.4 Decisão

```
Montanha     = SoR (desired, lease, assignment, CAS, 2PC)
Pedra local  = motor do fold e do SoR (dois papéis, um kernel)
Fold (novo)  = Slipstream-class: materializa prefixo, cursor, cache
               NÃO é membro Raft; NÃO faz get_strong
```

| Ideia | Veredito |
|-------|----------|
| “Montanha não suporta arquiteturalmente o teto Slipstream” | **Falso** se o teto é *leitura local*. **Verdadeiro** se o teto é “todo get passa por majority.” |
| “Precisamos de outra DB transacional em cima” | **Falso.** Segundo 2PC/Raft no caminho do request mata a escala. |
| “Precisamos de outro *produto* em cima (fold/cache)” | **Verdadeiro.** É o que QS v2 e Slipstream *são*. |
| “Trocar Montanha por Slipstream/NATS” | **Falso.** Jepsen + log bounded. SoR continua CP. |
| “Unbundlar log vs storage (FDB roles) para chegar no QS” | **Não agora.** RFC-0021 A até o p99 de *commit* provar o contrário. O teto QS não é commit. |

Ainda dá para aprender, *depois* do fold existir: working-set medido,
relay, prefetch reativo, keep-keys/evict-values, soft/hard disk,
instâncias (DNS ≠ WAF), heartbeat de lag, não descobrir a si mesmo.
Isso é o P2 da nota; não muda o SoR.

**RFC (draft):** [`rfc/0024-montanha-fold-for-caixote.md`](rfc/0024-montanha-fold-for-caixote.md).

## 9. Pointers

| Doc | Why |
|-----|-----|
| [`references/quicksilver/SOURCES.md`](references/quicksilver/SOURCES.md) | Pinned quotes + numbers |
| [`references/slipstream/`](references/slipstream/) | Raw README / ARCHITECTURE / BACKENDS / `protocol.rs` |
| [`nats-need-replacement.md`](nats-need-replacement.md) | JetStream ≠ fold; ack ≠ fsync |
| [`rfc/0019-local-primitive-for-platform-and-scylla-need.md`](rfc/0019-local-primitive-for-platform-and-scylla-need.md) | CAS / seq / CHANGELOG we already shipped |
| [`rfc/0021-geo-multiregion.md`](rfc/0021-geo-multiregion.md) | What “geo” currently is (dial only) |
| [`proxy-raft-replicas-proposal.md`](proxy-raft-replicas-proposal.md) | Client VIP ≠ peer identity (QS DNS lesson) |
| [`architecture-refined.md`](architecture-refined.md) | Pedra is local; cluster is a layer |
| [`object-storage-as-substrate-possibility.md`](object-storage-as-substrate-possibility.md) | Artifact transport belongs above Rung 0 |
