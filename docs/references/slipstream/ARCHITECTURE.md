# Slipstream Architecture

Takes NATS JetStream KV buckets as input and produces a local durable fold — a `SnapshotStore` whose cursor is guaranteed to name only revisions whose `apply` has returned — so edge nodes survive restarts without full replay, and can bootstrap from a content-addressed artifact when the NATS log has compacted past their cursor.

## Data Flow

### Happy path: startup with snapshot — no export

```
Disk ──► load(path) ──► replay_log() ──► HashMap<key, KvEntry> + WatchCursor
                                                    │
                                    watcher.watch_all_from(cursor, tx)
                                                    │
                                              CursorExpired?
                                             /              \
                                           Yes               No
                                            │                 │
                            stale-key resync (synthetic      delta stream
                            deletes) + watch_all(tx)              │
                            (state-sync re-list)                  │
                                            └───────┬─────────────┘
                                                    │
                                   KvUpdate → cache.apply() + snap.write_update()
                                                    │
                                     snap.checkpoint(cursor) ──► compact() if due
```

### Read path

```
reader.get("key")    ──► NATS kv.entry() ──► filter tombstones ──► KvEntry | None
reader.entry("key")  ──► NATS kv.entry() ──► raw (includes tombstones, for CAS)
reader.scan("pfx.")  ──► ephemeral push consumer (DeliverPolicy::LastPerSubject) ──► Vec<KvEntry>
reader.keys("pfx.")  ──► same consumer, headers_only ──► Vec<String>
```

### CAS write path

```
writer.create("lock", val)           ──► kv.create()        ──► AlreadyExists | VersionToken
writer.update("node", val, ver)      ──► kv.update()        ──► RevisionMismatch | VersionToken
writer.delete_with_version("k", ver) ──► kv.update(key, []) ──► RevisionMismatch | bool
```

CAS tombstone (empty-value Put) is how `delete_with_version` works — it writes an empty value via a CAS operation so concurrent writers see a conflict. `get()` and `scan()` filter these out; `entry()` exposes them for CAS callers that need the version.

### Bootstrap via artifact (bounded log, cursor evicted)

```
ObjectStore (<prefix>/<key>.manifest.json)
  ↓ download pointer
ExportManifest (cursor, backend, file hashes)
  ↓ derive payload key from blake3(manifest)[..8]
ObjectStore (<prefix>/<key>.payloads/<hash>.tar)
  ↓ stream to temp dir (verified: embedded manifest == pointer bytes, every file hash re-checked)
Staged artifact dir
  ↓ rename into place (atomic; bad artifact never becomes the fold)
SnapshotStore (recovered cursor = manifest.cursor)
  ↓ watch_applied(resume=cursor)
NATS JetStream (tail delta from cursor+1)
  ↓ apply tail
Converged fold
```

### Export round (one winner per fleet, TTL-gated)

```
ExportLease::try_acquire(ttl)
  ↓ create-only CAS wins; expired/corrupt lease CAS-stolen
watch_applied ExportRequest channel
  ↓ pending batch flushed first
store.export_to(artifact_dir)
  ↓ backend snapshot + verify-by-reopen (cursor equality gate)
ExportManifest (cursor, backend, file BLAKE3 hashes)
  ↓
ObjectStoreTransport::upload(key, artifact_dir)
  ├─ tar data/ → multipart upload (8 MiB chunks, 8 concurrent)
  │    key = <prefix>/<key>.payloads/<blake3(manifest)[..8]>.tar
  └─ swap_pointer() → conditional put at <prefix>/<key>.manifest.json
       ├─ pointer_publish_allowed(current, candidate_rank)?
       │    true  → CAS put (max 8 retries on concurrent swap)
       │    false → SupersededByNewer (slow exporter gives up cleanly)
       └─ PublishOutcome::Published | SupersededByNewer
  ↓ Published
LeaseGuard::complete(cursor)
  ↓
ObjectStoreTransport::prune(key, grace=4×ttl)
  └─ delete payloads: payload_prunable(payload_rank, pointer_rank, is_target, aged_out)?
  ↓
delete local artifact dir (transience enforced — not hoped for)
```

### Watch resumption

```
watch_all_from(cursor, tx)
  cursor.is_none() ─────────────────────────► watch_all(tx)        (full replay)
  cursor has rev ──► kv.watch_all_from_revision(rev+1)
                       │                       │
                    Success                CursorExpired (NATS compacted past cursor)
                       │                       │
                   delta stream          caller falls back to watch_all(tx)
```

Every non-`_from` watch is a **state-sync** stream (NATS `DeliverPolicy::LastPerSubject`): the current value of every matching key is delivered first — the re-list — then live updates. A no-cursor consumer therefore converges on full bucket state from the watch alone, with no separate scan and no scan-to-watch race window. The `_from` variants skip the re-list and deliver only the delta.

## Concepts & Terminology

| Term                     | Definition                                                           | NOT                                              |
| ------------------------ | -------------------------------------------------------------------- | ------------------------------------------------ |
| `Connection`             | Socket lifecycle manager + store factory                             | Not a store; not the NATS client itself          |
| `KvStore`                | Named bucket; vends reader, watcher, writer                          | Not the connection; holds no socket              |
| `KvReader`               | Point-in-time reads: `get`, `entry`, `keys`, `scan`                  | Not a live stream; returns a snapshot moment     |
| `KvWatcher`              | Live update stream pushed via mpsc channel                           | Not a polling loop; push from NATS               |
| `KvWriter`               | Write, soft-delete, CAS (`create`, `update`, `delete_with_version`)  | Not multi-key transactions                       |
| `WatchCursor`            | Opaque resume position in a watch stream (NATS: u64 revision)        | Not a per-key version; only for watch resumption |
| `VersionToken`           | Opaque per-key version (NATS: 8-byte u64; FDB: 10-byte versionstamp) | Not a wall-clock timestamp; not globally ordered |
| `KvEntry`                | One key + value + version from a read                                | Not a watch event; immutable once returned       |
| `KvUpdate`               | One watch event: `Put`, `Delete`, or `Purge`                         | Not a read result; carries deletes too           |
| `Snapshot`               | Deduplicated KV state + cursor persisted to disk                     | Not the source of truth; a cache of NATS         |
| `SnapshotWriter`         | Append-only log of `KvUpdate`s; no in-memory state beyond a counter  | Not the in-memory cache itself                   |
| `SnapshotStore`          | Trait: the durable-fold contract — atomic `apply(batch, cursor)`, `load`, `get`, `range` | Not a serving index; stops at fold + cursor + query |
| `AppendLogSnapshot`      | Default `SnapshotStore`: append-only log + in-RAM fold (pure-Rust)   | Not for folds larger than RAM                     |
| `FjallSnapshot`          | On-disk `SnapshotStore` (fjall LSM, `feature = "fjall"`) for large folds | Not in the pure-Rust core; opt-in feature        |
| `RocksDbSnapshot`        | On-disk `SnapshotStore` (RocksDB, `feature = "rocksdb"`) for large folds | Not pure-Rust; opt-in feature with a C++ build dep |
| `watch_applied`          | Combinator: batch → apply → *then* advance cursor / fold into `SnapshotStore`; resyncs stale keys on cursor expiry | Not a raw watch; the cursor follows `apply`, not receipt |
| `WatchScope`             | What `watch_applied` watches: `All`, `Prefix`, or `Prefixes` (multi-filter union) | Not N consumers; `Prefixes` costs one consumer    |
| `ConnectionCapabilities` | Feature flags for runtime branching (CAS, streaming watch, …)        | Not enforced; purely advisory                    |
| `ExportRequest`          | One-shot channel message that asks `watch_applied` to flush + export the current fold | Not a store operation; handled between flushes   |
| `ExportLease`            | Fleet-wide at-most-one coordinator: CAS key that prevents N nodes exporting the same round | Not a correctness gate; only dedup. Pointer monotonicity is the gate |
| `LeaseGuard`             | RAII guard for a won export round: `complete(cursor)` stamps success, `abandon()` frees it early via CAS delete | Not held across restarts; process crash → expiry → next node steals |
| `LeaseRecord`            | Value stored in the lease key: holder, acquired/expires timestamps (wall-clock, embedded — no server TTL), optional completed cursor | Expiry is compared by callers, not the store; requires NTP-sane clocks |
| `ExportManifest`         | Artifact metadata committed to disk and object store: backend identity, format generation, cursor, per-file BLAKE3 hashes | The manifest bytes are their own content address (blake3[..8]) |
| `ArtifactTransport`      | Trait: `upload(key, dir) → ManifestAndOutcome`, `download(key) → ManifestAndDir`, `pointer(key)`, `prune(key, grace)` | Not an object store; adapts `ObjectStore` for the pointer-swap protocol |
| `ObjectStoreTransport`   | Concrete `ArtifactTransport` over any `object_store` backend (S3, GCS, Azure, local) | `file://` lacks CAS and FAILS CLOSED unless `with_non_atomic_pointer_fallback()` (dev only) |
| `PublishOutcome`         | `Published` (pointer advanced) or `SupersededByNewer` (refused — a newer pointer exists) | Not an error; a slow exporter's normal exit under concurrent rounds |
| `PointerState`           | What `swap_pointer` observes before deciding: `Absent`, `Present { rank: Some(n) }`, or `Present { rank: None }` (corrupt) | Used only by `pointer_publish_allowed`; never stored independently |

## Layer Architecture

```
┌─────────────────────────────────────────────────────────────┐
│        KvReader │ KvWatcher │ KvWriter │ KvTtl              │
│         (async_trait, object-safe, Arc<dyn Trait>)          │
├─────────────────────────────────────────────────────────────┤
│                        KvStore                              │
│          (named bucket — vends the three roles above)       │
├─────────────────────────────────────────────────────────────┤
│                       Connection                            │
│         (connect/shutdown/is_healthy + store factory)       │
├─────────────────────────────────────────────────────────────┤
│                    NatsConnection                           │
│   NatsKvStore │ NatsKvReader │ NatsKvWatcher │ NatsKvWriterImpl
│              (concrete NATS JetStream impl)                 │
└─────────────────────────────────────────────────────────────┘
                  snapshot.rs (orthogonal, optional)
┌─────────────────────────────────────────────────────────────┐
│   SnapshotStore trait: apply(batch, cursor) │ load │ get │ range
│   AppendLogSnapshot (default, in-RAM)                       │
│   FjallSnapshot │ RocksDbSnapshot (feature-gated, on-disk)  │
│          (append-only CRC log, tempfile+rename compact)     │
└─────────────────────────────────────────────────────────────┘
                  applied.rs (combinator over KvWatcher + snapshot)
┌──────────────────────────────────────────────────────────────┐
│   watch_applied(): batch → apply → advance cursor/checkpoint │
│   (cursor-after-apply; the safe default for resumable watch) │
└──────────────────────────────────────────────────────────────┘
```

## Core Mechanism

### Resumable Watch

The cursor is the NATS stream sequence number at the last checkpoint. On restart, pass it to `watch_all_from()` to subscribe at `cursor+1` — only the delta arrives, not the full history.

When the cursor expires (NATS retention window evicted those records), `CursorExpired` is returned. The fallback `watch_all()` re-list re-delivers the current value of every live key, but it cannot cover keys **deleted during the gap whose delete markers were also evicted** — those need synthetic `Delete` events diffed from prior state. `watch_applied` does this automatically when given a reader (see below). A raw-API caller hand-rolls the same diff with `Snapshot::stale_keys()`:

```rust
match watcher.watch_all_from(&snap.cursor, tx).await {
    Ok(()) => {}
    Err(KvError::CursorExpired) => {
        let live = reader.keys("").await?;
        for key in snap.stale_keys(live.iter().map(|s| s.as_str())) {
            cache.remove(key);
        }
        watcher.watch_all(tx).await?;
    }
    Err(e) => return Err(e.into()),
}
```

### Applied-Cursor Watch (`watch_applied`)

The resumable watch above hands the caller raw machinery — a channel of `KvUpdate`s, a cursor, a snapshot writer — and trusts each one to hand-roll the loop that batches updates, applies them, and advances the cursor. Every hand-rolled instance got the same step wrong: it advanced the cursor on **receipt** of an update (`high_water = rev` at `rx.recv()`) and applied the batch afterward. The combinator `watch_applied` exists to encode the correct discipline once.

**The resume guarantee.** This library's contract is "resume from a sequence number after any restart." `watch_applied` sharpens that into a single invariant:

> A persisted/reported cursor `C` ⟹ every update with revision ≤ `C` has been **applied** — the caller's `apply()` has returned for it.

The cursor is written from `apply()`'s completion, never from the channel's delivery. Concretely, on each flush the combinator runs `apply(batch)` to completion, *then* sets `cursor = batch_high`, *then* checkpoints the snapshot at that cursor, *then* fires `on_applied`. Nothing advances the cursor before `apply` returns.

**Why receipt is the wrong signal.** Bumping the cursor at `rx.recv()` and applying later opens a crash window: the persisted cursor claims "caught up to rev N" while rev N still sits in an unapplied batch buffer (or in flight to a separate apply task). On crash+resume the watch re-arms at `cursor+1`, *past* the unapplied rev N, and silently skips it. The data is gone with no error — a hole in the exact guarantee the crate advertises.

This is the lesson of Saltzer, Reed & Clark, *End-to-End Arguments in System Design* (1984): a checkpoint placed below the endpoint — here, at the transport's delivery rather than at the application of the update — can only ever be a performance hint, never a correctness guarantee. The "it happened" property can only be established at the endpoint that actually performs the work, so the cursor is sourced from `apply`, not from `recv`. The cursor-as-monotonic-index shape itself is the HashiCorp Consul anti-entropy / blocking-query lineage: a client re-arms its watch from the last index it *reconciled*, never from the index it merely *saw*.

**Cursor authority covers rejected entries.** `batch_high` tracks the highest revision *received* since the last flush, including updates that `parse` rejected (corrupt bytes, irrelevant keys). A rejected entry is still "nothing to apply," so it is covered by the cursor — and because NATS delivers in revision order, advancing to the max revision after one atomic `apply` is sound: having seen the max means every revision below it has been seen too. Without this, a run of irrelevant keys would pin the cursor in place and force redundant replay on every restart.

The one exception: an update carrying the **unknown** version (an unparseable ACK subject on the hand-built multi-prefix consumer path) never touches `batch_high` — it can neither mint a cursor position nor clobber the real high from earlier in the batch. The update is still applied; skipping it under-advances at worst, and re-delivery on resume is idempotent. (The pre-guard code fabricated revision 0 for these, which the loop adopted as a real position — regressing the persisted cursor to 0 and forcing a full replay on the next restart.)

**Snapshot consistency.** Raw `KvUpdate`s stream to the snapshot log as they arrive, but the *checkpoint* cursor is the post-apply cursor. A crash after a raw record is written but before its `apply`/checkpoint leaves the log holding data *ahead* of its cursor — which is safe: the cursor never names a revision whose `apply` had not returned, so resume re-delivers and re-applies that tail rather than skipping it. Compaction runs off the hot path via `spawn_blocking`, as everywhere else in the snapshot subsystem.

**Flush triggers.** A batch flushes when any of these fires: the `window` elapses, `batch.len()` reaches `config.max`, a shutdown is signalled, or the channel closes with a pending batch (the remainder is flushed before returning).

**Transient store failures re-queue, not drop.** If `store.apply()` returns an error, the raw batch is prepended to the next flush's accumulation and the watch continues. The streak counter increments; at 16 consecutive failures the watch fail-stops with `KvError::WatchError`. Dropping the failed batch and continuing was the shipped behavior until `transient_store_failure_never_leaves_a_cursor_gap` reproduced the bug: a transient failure followed by a successful flush advanced the cursor over a hole that survived every restart, because the restart re-folds from the advanced cursor, skipping the missing range. Cursor authority requires the store's cursor and contents to advance together, always (`applied.rs:303–394`, `tests/model_applied.rs`).

**Cursor-expired resync.** On `CursorExpired` from the resume path the combinator falls back to the full-scope watch (`watch_all` / `watch_prefix` / `watch_prefixes`), whose state-sync re-list re-delivers every live key as puts. The re-list cannot cover keys deleted during the gap whose markers were evicted with the cursor, so — when the combinator is given a `KvReader` and a store — it closes that hole first: the watch task lists the bucket's live keys, hands them to the main loop, and waits for an ack; the main loop flushes, diffs the fold's in-scope keys against the listing, and runs a synthetic `KvUpdate::Delete` (unknown version — never advances the cursor) through `parse`/`apply`/store for each key that vanished; only then does the fallback watch start. That ack ordering is the invariant: a synthetic delete always precedes the re-list put for the same key, so delete-then-recreate during the gap converges. Without a reader the fallback is re-list-only and logs the possible stale keys.

This is the layer the tunnel router (swap route table) and edge origin watcher (rebuild hashrings) both collapse onto: `parse` extracts the domain registration, `apply` swaps the live state, `on_applied` persists the cursor.

### Live retention floor guard (`stream_watch_floor_guarded`, `nats.rs:1042`)

NATS does **not** error when a live consumer's position is overrun by retention — it silently skips evicted messages (the same silent-clamp behaviour that `resume_window_ok` closes at resume time). For an All-scope watch, a skipped delete marker permanently diverges the fold, with zero log lines. The floor guard closes this mid-stream:

- **In-band (primary):** when a delivered revision jumps the frontier by more than 1, fetch `first_sequence` and call `resume_window_ok(frontier, first_seq)`. A benign interior gap — per-subject overwrites below the floor threshold — passes. Head eviction past the frontier fails the watch into the restart → `CursorExpired` → resync repair path. The check runs *before* the entry is processed: the fold never advances past unexamined evidence of loss.
- **Backstop (30 s):** when no deliveries arrive, the periodic probe catches the no-traffic case where there is no in-band evidence to act on.

A periodic-only design was rejected by the model checker: deliveries catching the frontier up past a gap between probes erase the evidence, leaving permanent silent divergence with the guard running. The in-band check is what makes the design correct (`tests/model_live_watch.rs`).

Scope: this guard is sound only for the All-scope unfiltered resume watch, where every stream message is deliverable and a delivery gap implies a missed message. Prefix-scoped watches deliver sparse revisions — gaps from non-matching subjects are indistinguishable from eviction gaps client-side — and retain the retention-outlives-lag operating axiom.

### scan() and keys() via Ephemeral Push Consumer

Both use `DeliverPolicy::LastPerSubject` — one ephemeral push consumer delivers the latest value per key in a single streaming operation, rather than N sequential `get()` calls. `keys()` adds `headers_only: true` so no value bytes cross the wire.

The consumer is always `AckPolicy::None`. The default `AckPolicy::Explicit` stops delivery after `max_ack_pending` (1000) un-acked messages, silently truncating any bucket with >1000 keys.

The consumer is created with **subscribe-before-create** ordering: the inbox subscription is registered before the consumer exists, closing a race in async-nats ≤0.46 where early messages arrive before the subscription is ready.

### ACK Subject Format Parsing in scan()

Each message delivered by the `scan()` push consumer carries the KV revision in its JetStream ACK subject (the message's reply subject). The revision is the stream sequence number, and it sits at a field offset that varies by NATS server version:

```
Legacy (9 tokens):  $JS.ACK.<stream>.<consumer>.<delivered>.<stream_seq>.<consumer_seq>.<ts>.<pending>
Modern (11–12 tok): $JS.ACK.<domain>.<account>.<stream>.<consumer>.<delivered>.<stream_seq>.<consumer_seq>.<ts>.<pending>[.<token>]
```

The stream sequence sits at index 5 (legacy) or index 7 (modern). The final token is always `num_pending` (typically 0), which looks like a sequence but is not. The previous implementation took the last token and produced a wrong version on every scanned entry; the current parser reads from the front and branches on token count.

The implementation uses a fixed 8-element stack array for the first 8 tokens (no heap allocation per message). An A/B against the previous `Vec`-collecting approach measured **~3.1× speedup** — 1.59 ms → 0.51 ms per 10k ACK parses. See `benches/ack.rs`.

### 30-Second Operation Timeout

Every NATS operation is wrapped in `timed()` (30 s). Without it, a CLOSE_WAIT connection (half-dead TCP) parks `await`s forever — async-nats does not fail in-flight requests when the TCP layer goes dead. 30 s is generous for legitimate slow ops (JetStream stream sync, leader election) while still being debuggable.

### VersionToken: Inline Multi-Backend Versioning

`VersionToken` is a 10-byte inline buffer — no heap allocation. It covers all current backends without widening:

| Backend  | Encoding                 | `as_u64()` |
| -------- | ------------------------ | ---------- |
| NATS     | 8-byte big-endian u64    | `Some(rev)` |
| FDB      | 10-byte versionstamp     | `None`     |
| Unknown  | len=0                    | `None`     |

### Machine-Checked Protocol Kernels (`protocol.rs`)

Three pure-function guards live in `protocol.rs`. Production code and the Stateright exhaustive model checker (`tests/model.rs`) call the **same functions** — not two hand-synchronized copies. A change to any guard is re-verified against the full bounded state space on the next `cargo test --test model`. Mutation tests prove each guard is load-bearing: substituting a broken variant produces a counterexample.

**`pointer_publish_allowed(current: &PointerState, candidate_rank: u64) → bool`**

The monotonic pointer guard. Returns `true` for an open slot, a corrupt pointer, or a candidate at or above the existing cursor — and `false` exactly when the existing pointer is parseable and strictly newer.

```
Absent                  → true   (open slot)
Present { rank: None }  → true   (corrupt pointer is replaceable, not a wedge)
Present { rank: Some(n) }
  candidate >= n        → true
  candidate < n         → false  (stale publish refused)
```

Machine-checked as: _"published cursor never regresses."_

**`payload_prunable(payload_rank, pointer_rank, is_pointer_target, aged_out) → bool`**

The prune guard. A payload is deletable only when all hold: it is not the pointer's target, its rank is parseable AND strictly below the pointer's, and its age has cleared the grace period.

```
!is_pointer_target && aged_out && payload_rank.is_some_and(|r| r < pointer_rank)
```

Strictly-below (not `<=`) is the structural fix that makes dangling pointers impossible: `pointer_publish_allowed` refuses any candidate strictly below the pointer, and the pointer is monotone — so anything this guard deletes can never be successfully published afterward. The model checker found a dangling-pointer counterexample under an earlier age-only rule; this guard is the fix.

Machine-checked as: _"pointer target always fetchable"_ under zero-grace pruning.

**`resume_window_ok(revision: u64, first_sequence: u64) → bool`**

The cursor-expiry guard. Resume reads `revision + 1` onward; it is sound iff `first_sequence ≤ revision + 1`. NATS does not error on a below-head start — it silently clamps to `first_sequence`, skipping evicted delete markers with no fallback. This check is ours to make.

```
first_sequence ≤ revision.saturating_add(1)  → sound
first_sequence >  revision + 1               → CursorExpired
```

Machine-checked as: _"bootstrap never silently diverges."_ Empirically pinned by `tests/resync.rs::nats_silently_clamps_resume_below_first_seq`.

### Export Round State Machine

```
[Idle] ──ExportRequest──► [Acquiring]
                              │
                    try_acquire(ttl)
                         /         \
                      Won          Lost (another holder)
                       │               │
                [Exporting]        [Idle]
                       │
             flush pending batch
             store.export_to(dir)
             verify-by-reopen (cursor eq)
                       │
                 [Uploading]
                       │
              tar + multipart put
              pointer_publish_allowed?
              ├─ true  → CAS swap (≤8 retries)
              └─ false → SupersededByNewer
                       │
              Published    SupersededByNewer
                 │                │
          [Completing]      [Abandoning]
                 │                │
         complete(cursor)   abandon() → CAS delete
         prune(grace=4×ttl)
         delete local dir
                 │
            [Idle]
```

| State        | Entry Condition                          | On Failure                             |
| ------------ | ---------------------------------------- | -------------------------------------- |
| Acquiring    | `ExportRequest` received by watch task   | —                                      |
| Exporting    | Lease won (CAS create or takeover)       | Abandon lease; delete local dir        |
| Uploading    | Export + verify-by-reopen succeeded      | Abandon lease; delete local dir        |
| Completing   | `Published`                              | Prune/complete failures are non-fatal  |
| Abandoning   | `SupersededByNewer` or upload failed     | CAS delete failure is non-fatal        |

### watch_applied loop state transitions

| From | Trigger | Guard | What Actually Happens |
|---|---|---|---|
| Watching | `rx.recv()` delivers a `KvUpdate` | version not unknown | `batch_high` updated; raw update pushed to `raw_batch`; `parse()` called; window timer armed on first update |
| Watching | `rx.recv()` delivers an unknown-version `KvUpdate` | — | Same, except `batch_high` untouched — an unparseable revision neither mints nor clobbers a cursor position |
| Batching | Window elapses | — | `apply(batch)`; cursor = `batch_high`; `store.apply(raw_batch, cursor)` on blocking task; `on_applied` fired |
| Batching | `batch.len() == config.max` | — | Same as window flush |
| Flush | `store.apply()` succeeds | streak < 16 | Streak reset; `on_applied` fired; continue watching |
| Flush | `store.apply()` transient error | streak < 16 | Raw batch prepended to next `raw_batch`; streak++; warn; continue |
| Flush | `store.apply()` error | streak ≥ 16 | `KvError::WatchError` returned; watch fail-stops |
| Watching | Watch task returns `CursorExpired` | first_seq > cursor+1 | Resync (if reader wired): list keys → synthetic deletes → ack; then `watch_all()` fallback |
| Resync | `reader.keys()` fails | — | Fatal: `KvError::WatchError`; restart re-runs the full resume → expiry → resync path |
| Resync | Fold range fails | — | Fatal: same |
| Watching | `GuardTrip` (in-band gap or 30s backstop) | `!resume_window_ok(frontier, first_seq)` | Watch task fails; `KvError::WatchError` surfaces; caller's restart hits `CursorExpired` and runs resync |
| Watching | `ExportRequest` | — | Flush pending batch; `store.export_to()` on blocking task; reply on oneshot; continue |
| Any | Shutdown signal | — | Flush pending batch; abort watch task; return applied cursor |
| Any | Channel closes cleanly | — | Flush remaining batch; return applied cursor |

## Snapshot Subsystem

### The durable-fold trait

The durable fold is a trait, `SnapshotStore`, so the backend is pluggable while the contract is fixed:

```rust
fn load(path) -> (WatchCursor, Self);          // resume position + store
fn apply(&mut self, batch, cursor);            // fold data AND advance cursor, atomically
fn get(key) -> Option<KvEntry>;                // point query
fn range(prefix) -> Vec<KvEntry>;              // ordered prefix scan
```

Three invariants bind every implementation:

- **Pure function of the log.** Delete the store, replay every update with revision `> cursor`, and the state is identical. The store caches the fold; NATS is the source of truth.
- **Cursor-after-apply.** `apply` makes data and cursor durable together, so the cursor never names a revision whose data is absent — one transaction on a transactional backend, data-then-cursor on the append log (a torn write leaves data *ahead* of the cursor, which replay re-folds, never skips).
- **Snapshot is a cache.** A tail lost to power loss (under a no-sync durability mode) is rebuilt by resuming the watch from the recovered cursor.

`watch_applied` is generic over `SnapshotStore`: on each flush, after `apply` returns, it hands the raw batch + post-apply cursor to `store.apply(...)` on a blocking task. The trait stops at fold + cursor + query; serving structures built from the fold (routing rings, hashrings) live in the consumer, which reads them out via `get`/`range`.

| Backend | Module | State | Durability |
| ------- | ------ | ----- | ---------- |
| `AppendLogSnapshot` (default) | `snapshot.rs` | Append-only CRC log + in-RAM `HashMap` fold | `checkpoint` flush (page cache); `fsync` only at `compact` |
| `FjallSnapshot` (`feature = "fjall"`) | `snapshot_fjall.rs` | On-disk fjall LSM (`data` + `meta` partitions) | One atomic batch per `apply` (data + cursor); per-commit `fsync` configurable (NO_SYNC default) |
| `RocksDbSnapshot` (`feature = "rocksdb"`) | `snapshot_rocksdb.rs` | On-disk RocksDB (`data` + `meta` column families), tuned for billion-key folds (hit-optimized ribbon filters, partitioned index, zstd bottommost — see the module's Tuning docs) | One atomic `WriteBatch` per `apply` (data + cursor); WAL always on, per-commit `fsync` configurable (NO_SYNC default) |

The two LSM backends are interchangeable in contract and share the value-record codec (`snapshot_record.rs`); fjall keeps the crate pure-Rust, RocksDB trades a C++ build dependency for the battle-tested engine and its operational tooling (`ldb`, `sst_dump`). Both keep the cursor in the same atomic batch as the data it names, so under NO_SYNC a crash can lose the un-synced tail but never desynchronize cursor from data — on reopen the recovered cursor is consistent and the watch re-folds the tail. The rest of this section describes the **append-log backend** (the default), whose on-disk format is below.

### File Format

```
Header:  b"PGSS" ++ version:u16le

Record:  crc32:u32le ++ type:u8 ++ payload

Put:     key_len:u16le ++ key ++ value_len:u32le ++ value ++ ver_len:u8 ++ version_bytes
Delete:  key_len:u16le ++ key ++ ver_len:u8 ++ version_bytes
Cursor:  cur_len:u8 ++ cursor_bytes
```

Version bytes are stored as length-prefixed raw bytes, not a fixed `u64`. A 10-byte FDB versionstamp round-trips intact; a `u64`-only field would flatten it to 0 and break every subsequent CAS on a restored entry.

CRC covers from the type byte through the end of the record. A truncated final record (crash mid-write) is silently discarded. A CRC mismatch in the middle of the file returns `SnapshotError::Corrupted`.

### State Machine

```
APPENDING ──► checkpoint() returns true ──► NEEDS_COMPACT
    │                                              │
write_update()                           compact() [blocking: replay → dedup → tempfile → rename]
    │                                              │
    └──────────────────────────────────────────────┘
                bytes_since_compact = 0
```

| From          | Event                            | To            | Guard / Side-effect                              |
| ------------- | -------------------------------- | ------------- | ------------------------------------------------ |
| APPENDING     | `write_update()`                 | APPENDING     | Buffered; bytes_since_compact += n               |
| APPENDING     | `checkpoint()` → true            | NEEDS_COMPACT | Cursor + cursor record flushed to page cache     |
| APPENDING     | `checkpoint()` → false           | APPENDING     | Same flush; below threshold                      |
| NEEDS_COMPACT | `compact()` succeeds             | APPENDING     | Tempfile → sync_all → rename; counter reset to 0 |
| NEEDS_COMPACT | `compact()` fails on reopen      | POISONED      | `writer = None`; subsequent writes return `Io`   |
| POISONED      | any `write_update`/`checkpoint`  | POISONED      | `Err(Io("snapshot writer poisoned"))` returned   |

### Load + Compaction

`load()` replays the full log into a `HashMap` (last write wins per key, deletes remove entries), then rewrites to a compact file (no duplicates) via tempfile + `sync_all` + rename. It skips the rewrite when the log is already compact (no duplicate keys, no delete records, clean EOF).

`compact()` flushes the BufWriter first so un-checkpointed records survive. It reads the current file, replays it, writes to a same-directory tempfile (same filesystem = atomic rename, no `EXDEV`), `sync_all`s, then renames.

`checkpoint()` writes only a cursor record and calls `BufWriter::flush()` — a `write(2)` into the page cache. This survives a process crash but NOT a power loss. The only `fsync` is in `compact()`. The snapshot is a cache; a lost tail is rebuilt from a NATS scan + watch replay.

### Export Lease (`export_lease.rs`)

`ExportLease` coordinates fleet-wide at-most-one export rounds using a NATS KV key as a mutex, with expiry embedded in the value (no server TTL machinery required).

**Acquisition:**
- `create(key, value)` → wins if key absent; exactly one caller fleet-wide succeeds
- If key exists: parse `LeaseRecord` → compare `expires_at` to wall clock
  - Expired or unparseable → CAS `update(key, new_record, observed_version)` to steal
  - Live → return `None` (another holder is active)

**`LeaseRecord` wire format (JSON):**
```json
{ "holder_id": "node-42", "acquired_at_unix": 1718000000, "expires_at_unix": 1718000300,
  "completed_cursor_hex": null, "completed_at_unix": null }
```
After upload: `completed_cursor_hex` and `completed_at_unix` are filled in by `complete(cursor)`.

**`LeaseGuard` RAII discipline:**
- `complete(cursor)` → CAS-rewrite lease with cursor + timestamp (fleet-visible last-export record)
- `abandon()` → CAS-delete the key to free the round early (next trigger elects immediately)
- Drop without completing → lease expires naturally; next node steals after TTL

**Why embedded TTL over server TTL:** Portable to any `KvWriter`; no version/bucket-flag requirements. The cost is wall-clock comparison — acceptable because a premature steal merely produces a duplicate artifact (last-write-wins on the same key), never corruption. The lease is work-dedup, not a correctness gate.

**Why corrupt-lease stealing:** One unparseable value would otherwise wedge the fleet at `expires_at` forever. The same rule as `pointer_publish_allowed`'s treatment of corrupt pointers: unknown/corrupt state is replaceable, not a hard stop.

### Export / Import (replica bootstrap)

When the watched bucket is a **bounded** log (size-capped, history evicted), a fold is no longer rebuildable from NATS alone — the folds become the only full replicas. Export/import makes them transferable, which is what lets a new node, a node with a lost/corrupt fold, or a node whose cursor aged out of the log bootstrap at all.

**Artifact anatomy.** A directory: the backend's files under `data/`, plus `MANIFEST.json` carrying the artifact schema version, the backend identity and its on-disk format generation, per-file sizes + BLAKE3 digests, and — the load-bearing field — the **watch cursor the payload is exactly consistent with**. Import resumes the watch from that cursor and replays only the log tail. The manifest is written last and the whole stage is atomically renamed, so an artifact that exists is complete; a crash mid-export leaves only a hidden temp dir.

**The cursor-consistency invariant.** `export_to(&mut self)` cannot run concurrently with `apply` (exclusive borrow), and inside `watch_applied` exports run between flushes via the `ExportRequest` channel (pending batch flushed first) — so the embedded cursor equals the applied cursor, exactly. Every backend re-proves it at export time by **reopening the copy** and checking cursor equality: because every `apply` commits the cursor in the same atomic batch as its data, a recovered cursor that matches the live one is a complete tail-loss detector.

**Per-backend export mechanics.**

- **append-log**: write a compacted log from the in-RAM fold (`compact_to_file`), verify by `load()`.
- **RocksDB**: the engine's native `Checkpoint` (memtable flush + SST hardlinks) — consistent by construction; verify-open anyway for the uniform guarantee.
- **fjall** (no checkpoint API): `persist(SyncAll)` (journal complete), best-effort quiesce (rotate memtables, drain flushes/compactions, bounded), copy the DB dir — hardlink immutable `tables/`/`blobs/`, byte-copy journal + metadata + `lock` — with a bounded retry against background GC, then verify-by-reopen. Correctness rests on the exclusive borrow + the verify gate, never on the quiesce.

**Import** stages a verified copy beside the destination (every hash re-checked — the transport that delivered the artifact is untrusted), opens the staged copy and gates on manifest-cursor equality, then atomically renames. A bad artifact never becomes a fold; a crash mid-import leaves nothing at the destination.

**Storage accounting.** Exports are hardlink-dominant (extra disk ≈ journal + small metadata, not 2×), but a lingering artifact pins files the source later compacts away — artifacts are **transient**: upload, then delete (`run_export_round` enforces this). Stage and destination must be on the fold's filesystem; the EXDEV fallback silently degrades hardlinks to full copies.

**Fleet coordination.** `ExportLease` makes exactly one replica perform a given export round: create-only CAS to win, expiry embedded in the value (no server TTL machinery), CAS takeover of expired/corrupt leases, `complete()` publishes the exported cursor on the key (the fleet-visible last-export record), `abandon()` frees a failed round early. Clock skew at worst causes a duplicate export — safe; the lease is dedup, not a correctness gate.

**Transport** (feature `transport`): `ObjectStoreTransport` adapts any `object_store` backend (S3, GCS, Azure, local). Object layout:

```
<prefix>/<key>.payloads/<blake3(manifest)[0..8] hex>.tar   ← content-addressed payload (write-once)
<prefix>/<key>.manifest.json                               ← monotonic pointer (published last)
```

The payload key is derived from the manifest bytes, making collisions require hash breakage. The manifest is the single trusted object; the payload key is derived from it at download time — readers never trust a separately-stored pointer to the payload.

`run_export_round` composes the full at-most-once round: lease → export (via `ExportRequest`) → upload → complete → prune → delete-local. Per-backend `import_remote` composes: download pointer → derive payload key → download + verify → import.

## Connection Lifecycle

```
NEW (healthy=false, handle=None)
    │
    │ .connect()
    ▼
CONNECTED (healthy=true, handle=Some(NatsHandle))
    │                  │
    │ .shutdown()      │ .store() → NatsKvStore
    ▼                  │ .is_healthy() → AtomicBool::load (O(1), no lock)
SHUTDOWN (healthy=false, handle=None)
    │
    └─► .connect() can reconnect
```

`is_healthy()` for the `new()` + `connect()` path reads an `AtomicBool` driven by an installed NATS event callback (`Connected`/`Disconnected`). For the `from_client()` path (pre-connected client, no event callback), it reads the client's live `connection_state()` instead.

The double-check pattern in `connect()` guards a concurrent connect race: a second caller that wins the dial drops its handle (leaving `installed=false` on the event callback) so the teardown event does not clobber the winner's `healthy` flag.

## Design Decisions

### Why a `watch_applied` combinator instead of leaving the loop to callers?

The raw `KvWatcher` + `WatchCursor` + `SnapshotWriter` pieces let callers hand-roll the batch/apply/advance loop — and every known caller advanced the cursor on *receipt* rather than after *apply*, silently skipping un-applied updates on crash+resume. That is a footgun in the library's core guarantee, not a caller bug to be fixed N times. Encoding cursor-after-apply once, behind a combinator that callers can't get wrong, is cheaper and safer than documentation. `apply` stays the only domain logic; the cursor/snapshot/`on_applied` bookkeeping is the library's. See [Applied-Cursor Watch](#applied-cursor-watch-watch_applied).

### Why do watches deliver current state first (state-sync semantics)?

async-nats's bare `watch`/`watch_all`/`watch_many` ride `DeliverPolicy::New` — live updates only. A consumer built on that needs a separate `scan()` to seed, and any write landing between the scan and the watch attach is in neither — silently lost until the next reseed (the seed-then-watch race, demonstrated in `watch_prefix_relist_covers_seed_then_watch_gap`). Mapping every non-`_from` watch to `_with_history` (`LastPerSubject`) makes the watch itself deliver the seed, in revision order, with no race window: one primitive, correct by construction. The cost is a full re-list on every no-cursor watch start — which is what a no-cursor start *means*; consumers that have state resume with a `_from` variant and skip it.

### Why does the cursor-expired resync list keys instead of re-scanning values?

The fallback watch's re-list already carries every live key's value, so the resync only needs to learn which keys *no longer exist* — `reader.keys()` (headers only, no value bytes) is sufficient and cheap. The fold side of the diff is equally key-only: it streams via `for_each_in_range` rather than `range()`, so an All-scope resync against an on-disk backend never materializes the whole fold (values included) on the repair path. The synthetic deletes carry an unknown version and never advance the cursor: the fold's persisted cursor stays at its (expired) position until real re-list revisions move it, so a crash mid-resync just re-runs the same idempotent diff on the next start. Ordering, not versioning, provides correctness: the resync acks before the fallback watch is established, so a synthetic delete can never land after the re-list put that resurrects the same key.

### Why KvError: Clone instead of Box<dyn Error>?

A failed connect future may be observed by multiple concurrent callers waiting on a shared result. `Clone` lets the error fan out to N waiters without `Arc`. The cost: `std::io::Error` and `async-nats` error types are not `Clone`, so their structured cause chain is flattened into a pre-rendered `String` at the boundary. The trade-off is explicit: no `#[source]` chain, but the message carries context instead.

### Why object-safe async traits (Arc<dyn Trait>) instead of generics?

`KvStore` vends `Arc<dyn KvReader>` / `Arc<dyn KvWatcher>` / `Arc<dyn KvWriter>` so callers hold narrowed capabilities without knowing the backend type at compile time. This lets services swap NATS for an in-memory stub in tests, and lets the edge proxy hold only `Arc<dyn KvReader>` without dragging in write types. The `async_trait` macro desugars to `Pin<Box<dyn Future>>` to satisfy object safety.

### Why optional watcher() and writer()?

Not all backends support streaming watch or writes. Optional returns (`Option<Arc<dyn …>>`) let the read path — the hot path for config consumers — be free of watch/write complexity. The edge proxy, for example, only calls `reader()`. Callers check `ConnectionCapabilities` to branch on feature availability before attempting optional paths.

### Why a raw JetStream API fallback for bucket creation?

Synadia Cloud returns `$JS.API.STREAM.CREATE` response shapes that `async-nats`'s `create_key_value()` cannot parse. The `create_kv_bucket_raw()` fallback sends the JSON config as a plain request/reply and classifies responses by error code:

- `10058` in `code` or `err_code` → stream already exists (non-fatal)
- `400` + "maximum number of streams" → Synadia at-limit but bucket may exist (non-fatal)
- Anything else → hard failure

The standard async-nats path is tried first; raw is a fallback — taken on a parse-shaped error **or a timeout**. A timeout must not skip the fallback: Synadia Cloud, the deployment the raw path exists for, is also the most likely to be slow or distant, and the raw path's own per-op timeouts bound a genuinely dead connection anyway.

### Why subscribe-before-create in scan/keys?

async-nats ≤0.46 has a race: the server can deliver the first batch of push-consumer messages before the client's subscribe call completes. Creating the consumer first loses those early messages. Subscribing to the inbox first closes the race — the subscription is ready before the server can deliver.

### Why checkpoint() does not fsync?

Checkpoints are frequent (every N watch events). An fsync per checkpoint would add milliseconds of disk-sync latency to the hot watch path. Since the snapshot is a cache backed by NATS, a tail lost to power loss is rebuilt from a NATS replay — not a correctness failure. The only `fsync` is in `compact_to_file()`, where it guarantees the new compact file is durable before the atomic rename replaces the old one.

### Why raw backend-dir artifacts instead of a logical re-encode?

Export copies the backend's own files rather than re-encoding entries into a portable format. The engine's native integrity machinery (fjall journal CRCs + version checksums, RocksDB MANIFEST validation) then verifies the artifact on open — the same code path that validates the fold after a crash — and hardlinks make export cost ~O(metadata) instead of O(data). The trade is per-backend artifact formats; the manifest's backend identity + format generation make that explicit, and the engines' own format markers travel inside the payload as defense in depth.

### Why does export verify by reopening the copy?

fjall has no checkpoint API, so its copy is assembled from parts; rather than trusting that assembly, every backend opens the staged copy and requires the recovered cursor to equal the live fold's. Cursor-in-every-apply-batch makes this a complete tail-loss detector, and it reuses the engine's recovery as the oracle instead of reimplementing consistency checks. The cost — one extra open per export — is noise for a periodic exporter.

### Why is the export lease's expiry in the value instead of a server TTL?

Per-message TTLs need a new-enough NATS server, a bucket flag, and a backend that supports them; an `expires_at` inside the value works on any `KvWriter` and keeps acquisition/takeover as two plain CAS operations. The cost is wall-clock comparison across nodes — acceptable because a premature steal merely produces a duplicate artifact (last-write-wins on the same key), never corruption: the lease is work-dedup, not a correctness gate.

### Why a content-addressed, write-once payload + monotonic pointer instead of two fixed-key objects?

The legacy layout wrote the payload at `<key>.payload.tar` and the manifest at `<key>.manifest.json` as independent last-write-wins objects. The model checker proved two failure modes under that layout: (1) a cursor regression when a slow exporter's stale manifest overwrites a faster one's, and (2) a dangling pointer when the manifest advances past a payload that an older concurrent publisher still holds. The shipped layout fixes both structurally:

- **Content-addressed payload** — the key is `blake3(manifest)[..8].tar`. Two different artifacts can never share a key. Re-uploading the same artifact is an idempotent overwrite. A slow exporter's payload is valid until pruned; it just can't become the pointer.
- **Monotonic pointer via CAS** — `pointer_publish_allowed` refuses any candidate strictly below the current pointer. Combined with CAS (no torn reads between observe and write), the pointer is monotone non-decreasing. Slow exporters always get `SupersededByNewer`, never succeed at regression.
- **Strictly-below prune rule** — `payload_prunable` only deletes ranks strictly below the pointer. Because the pointer is monotone and `pointer_publish_allowed` refuses `< pointer`, a pruned payload can never become reachable again. The model checker found the dangling counterexample under an earlier age-only rule; this is the structural fix.

Machine-checked as three theorems in `tests/model.rs`: published cursor never regresses, pointer target always fetchable (zero-grace prune), bootstrap never silently diverges.

### Why extract protocol guards into `protocol.rs` and share them with the model?

Production code and an exhaustive model checker running the same logic closes the drift gap between proof and implementation. Previously the model had its own inline guard copies; a change to the production guard would not update the model, leaving the proof covering the old variant. Extracting the three guards into `protocol.rs` and importing them from both call sites means every `cargo test --test model` verifies the current production code, not a snapshot. The cost is a hard dependency: `tests/model.rs` imports `pub mod protocol` directly, so the module cannot go private.

### Why write in sorted key order during compaction?

`HashMap` iteration order is random per process. Sorting produces a deterministic byte layout for a given logical state, enabling byte-level snapshot comparison (integrity checksums, test assertions) and making file diffs readable. The O(n log n) sort is negligible relative to the I/O it precedes.

## NATS Mapping

| Concept                  | NATS primitive                                                                  |
| ------------------------ | ------------------------------------------------------------------------------- |
| `KvStore`                | JetStream KV bucket (`KV_{name}` stream, `$KV.{name}.>` subjects)              |
| `VersionToken`           | Per-key stream sequence number (u64, stored big-endian in the 8-byte token)    |
| `WatchCursor`            | Stream sequence number at last checkpoint                                       |
| `delete()`               | `kv.delete()` — writes `KV-Operation: DEL` marker; always returns `Ok(true)`   |
| `delete_with_version()`  | `kv.update(key, [], rev)` — CAS write of empty value as tombstone               |
| `KvUpdate::Purge`        | `KV-Operation: PURGE` — all history removed; treated same as Delete in snapshot |
| `scan()` / `keys()`      | Ephemeral push consumer with `DeliverPolicy::LastPerSubject`                    |
| `watch_all()`            | `kv.watch_with_history(">")` — `LastPerSubject`: state-sync re-list, then live |
| `watch_prefix()`         | `kv.watch_with_history("{prefix}>")` — server-side subject filter, same re-list |
| `watch_prefixes()`       | `kv.watch_many_with_history([..])` — ONE multi-filter consumer (NATS 2.10)      |
| `watch_all_from(cursor)` | `kv.watch_all_from_revision(cursor+1)` — server-side delta delivery            |
| `watch_prefixes_from()`  | Hand-built ordered push consumer: `filter_subjects` + `ByStartSequence(cursor+1)` (async-nats has no `watch_many_from_revision`) |

## Trust Model

**What the store layer verifies:**
- NATS credentials are valid (at `connect()`)
- Bucket exists or can be created (at `store()`)
- Snapshot CRC per record (at `load()` and `compact()`)
- Snapshot magic bytes and format version (at `load()`)
- Resume window soundness — `resume_window_ok(revision, first_sequence)` before attaching (NATS silently clamps; we error explicitly)

**What passes through unchecked:**
- Key names (no validation; NATS accepts any key)
- Value content (raw bytes; deserialization is the caller's responsibility)
- Bucket permissions (NATS auth rules govern access; the store layer does not re-check)
- Channel capacity (caller sets it; a full channel backpressures the watcher task)
- Snapshot cursor validity (stale cursors surface as `CursorExpired` from NATS, not from the snapshot layer)

**Why this is acceptable:** Applications own value encoding (JSON, proto, etc.). NATS owns authorization. The store layer is a transport adapter.

**What the transport layer verifies (import path):**
- Embedded manifest in the tar matches the pointer bytes downloaded from object store (content-address cross-check)
- Every payload file's BLAKE3 hash matches the manifest (per-file integrity)
- Recovered cursor of the staged copy equals the manifest cursor (tail-loss detector)
- Backend identity + format generation match the local node's backend

**What the transport layer does NOT verify:**
- Object store authorization (delegated to `object_store` credentials)
- Pointer freshness (callers decide whether the embedded cursor is recent enough)
- Whether the exporting node's fold was correct (garbage in, garbage out — the integrity guarantees cover transport, not source correctness)

## Failure Modes

| Failure                              | What Actually Happens                                                       | Recovery                                          |
| ------------------------------------ | --------------------------------------------------------------------------- | ------------------------------------------------- |
| Transient `store.apply()` failure    | Raw batch re-queued at front; next flush commits cumulatively; streak counter incremented | Automatic up to 16 consecutive failures |
| 16 consecutive `store.apply()` failures | Watch fail-stops: `KvError::WatchError` returned | Restart re-folds from last good cursor; investigate store (disk, permissions) |
| Live watch retention overrun (All scope) | Floor guard detects in-band (gapped delivery) or via 30 s backstop; watch fails with `KvError::WatchError` | Caller restarts; resume hits `CursorExpired`; resync repairs stale keys |
| Live watch retention overrun (Prefix scope) | **Undetected until the next restart** — prefix watches deliver sparse revisions, so eviction gaps are indistinguishable from non-matching subjects client-side; the floor guard is sound for All scope only (a periodic probe here would false-positive into spurious resyncs, a design the model checker rejected) | Next restart's resume-window check hits `CursorExpired` → resync repairs; operate prefix-scoped watches with retention window comfortably above the restart/redeploy interval |
| Resync `reader.keys()` failure       | Watch fail-stops (fail-stop, not degrade — degrade semantics proven to leave stale keys permanently) | Restart re-runs full resume → expiry → resync |
| `CursorExpired`                      | `watch_applied` lists live keys, diffs fold, applies synthetic deletes, then falls back to state-sync re-list | Automatic; raw callers use `stale_keys()` manually |
| `WatchError`                         | Watch stream dropped (NATS restart, reconnect)                              | Re-subscribe                                      |
| `Timeout` on any NATS op             | CLOSE_WAIT half-dead TCP parks the `await` without this guard              | Call `shutdown()` + `connect()`                   |
| `RevisionMismatch` on CAS            | Concurrent writer won the race                                              | Re-read with `entry()`, resolve, retry            |
| `AlreadyExists` on `create()`        | Key already present; caller's create was not exclusive                      | Read live value, decide whether to proceed        |
| Snapshot truncated tail              | `load()` discards partial final record; earlier records intact              | Resume from recovered cursor; tail re-folded      |
| Snapshot mid-file CRC mismatch       | `SnapshotError::Corrupted`                                                  | Delete snapshot, full NATS replay                 |
| Snapshot wrong format version        | `SnapshotError::InvalidFormat`                                              | Delete snapshot, full NATS replay                 |
| `compact()` I/O error                | Writer poisoned; subsequent writes return `Io`                              | Delete snapshot, rebuild from NATS                |
| Synadia Cloud stream limit           | Raw API path treats as non-fatal; verifies bucket with `get_key_value`      | Non-fatal if bucket exists                        |
| Crash between payload upload and pointer swap | Old pointer remains fully consistent; payload orphaned | Next export round publishes new pointer; stale payload pruned after grace |
| Slow exporter after newer round published | `pointer_publish_allowed` returns false → `SupersededByNewer`           | Lease abandoned, local artifact deleted; payload orphaned until prune |
| Tampered / torn artifact             | `ArtifactInvalid` at import (hash + cursor gate); nothing written to destination | Fetch another artifact                        |
| Artifact backend/format mismatch     | `ArtifactInvalid` before any open; engine format markers re-checked internally | Fetch another artifact                        |
| Export under churn — fjall copy      | File GC'd mid-copy; retry ×3; verify-by-reopen catches anything torn       | Abandon lease on persistent failure               |
| Export/upload fails mid-round        | Lease abandoned (CAS delete); local artifact deleted                       | Next trigger elects a new node                    |
| Artifact cursor older than NATS log  | `CursorExpired` on resume → full watch fallback + stale-key resync          | Checkpoint more often than log retention window   |
| Corrupt lease value                  | Treated as expired: CAS-stolen by next acquirer                             | Non-fatal; one bad write cannot wedge the fleet   |
| `prune()` I/O error                  | Stale payloads linger; warning logged                                       | Retried next export round; correctness unaffected |

## Package Structure

| File                       | What It Does                                                                         |
| -------------------------- | ------------------------------------------------------------------------------------ |
| `src/kv.rs`                | Core traits (`KvReader`, `KvWriter`, `KvWatcher`, `KvTtl`) and types (`KvEntry`, `KvUpdate`, `VersionToken`, `WatchCursor`, `KvError`) |
| `src/stores.rs`            | `Connection`, `KvStore`, `StoreConfig`, `StorageType`, `ConnectionCapabilities`      |
| `src/nats.rs`              | NATS JetStream implementation; bucket creation, scan consumer lifecycle, timeout wrapping, Synadia Cloud workarounds, `check_resume_window` |
| `src/protocol.rs`          | Pure-function protocol guards: `pointer_publish_allowed`, `payload_prunable`, `resume_window_ok` — called by both production code and the Stateright model |
| `src/snapshot.rs`          | `SnapshotStore` trait; append-only log + `AppendLogSnapshot` (default backend): `SnapshotWriter`, `load()`, `replay_log()`, `compact_to_file()` |
| `src/snapshot_fjall.rs`    | `FjallSnapshot`: on-disk `SnapshotStore` backed by fjall (`feature = "fjall"`)       |
| `src/snapshot_rocksdb.rs`  | `RocksDbSnapshot`: on-disk `SnapshotStore` backed by RocksDB (`feature = "rocksdb"`) |
| `src/snapshot_record.rs`   | Shared `[ver_len][version][value]` value-record codec for the LSM backends           |
| `src/artifact.rs`          | `ExportManifest`, `ArtifactFile`, BLAKE3 integrity; stage-then-rename discipline; backend `export_to` + `import` (append-log, fjall, RocksDB) |
| `src/export_lease.rs`      | `ExportLease`, `LeaseGuard`, `LeaseRecord`: fleet-wide at-most-one via embedded-expiry CAS |
| `src/transport.rs`         | `ObjectStoreTransport`, `ArtifactTransport` trait, `run_export_round`: monotonic pointer swap, multipart upload, prune, content-addressed keys (`feature = "transport"`) |
| `src/applied.rs`           | `watch_applied` cursor-after-apply combinator, generic over `SnapshotStore`: `WatchScope`, `BatchConfig`, cursor-expired stale-key resync, `ExportRequest` handling |
| `src/lib.rs`               | Re-exports all public types; no logic                                                |
| `benches/`                 | Criterion benchmarks: snapshot write/checkpoint/load throughput, batch throughput, ACK subject parsing |
| `tests/integration.rs`     | NATS JetStream backend integration suite: each test boots its own `nats-server` on a free port; covers bucket create, CAS, watch semantics, cursor resume, and delete reconciliation |
| `tests/snapshot_store.rs`  | Backend-agnostic `SnapshotStore` conformance suite: every check is generic over the backend, instantiated for `AppendLogSnapshot`, `FjallSnapshot`, and `RocksDbSnapshot`; new backends get the whole suite for free |
| `tests/transport.rs`       | Integration: upload/download/manifest round-trip, pointer swap, prune, non-CAS fail-closed |
| `tests/transport_s3.rs`    | Live MinIO: CAS semantics (create, If-Match update, refusal, crash-window availability, prune) verified against a real object store |
| `tests/bootstrap.rs`       | Tier-2 bootstrap proofs on live NATS + on-disk backends: exports from a live `watch_applied` loop under churn, imports as a second node, and asserts **delta-only resume** via delivery count (not just convergence — a full replay would produce the same end state) |
| `tests/multi_export.rs`    | Prevention proofs on live NATS + real fjall/RocksDB: slow-exporter clobber refused, crash window keeps bootstrap available, multi-SST post-compaction fidelity |
| `tests/resync.rs`          | Live-NATS conformance: NATS silent-clamp pinned; full expiry → resync chain e2e; no-reader divergence pinned |
| `tests/model.rs`           | Stateright exhaustive model (~250M states, fleet size 2–3, unbounded rounds): pointer swap theorems (no regression, no torn pair, no dangling pointer, no silent divergence, terminal liveness); mutation tests prove each protocol guard load-bearing |
| `tests/model_applied.rs`   | Stateright cursor-authority model: delivery → flush → transient failure → crash/restart; `DropFailedBatch` and `ResumeFromMemApplied` mutations pin both pre-fix bug classes |
| `tests/model_resync_order.rs` | Stateright resync ack-barrier model: proves synthetic deletes strictly precede re-list puts; `NoAckBarrier` mutation reaches lost-recreate divergence |
| `tests/model_live_watch.rs` | Stateright floor-guard model: guarded variant proves fold always converges; unguarded variant pins permanent silent divergence as the machine-checked record of the pre-guard code |
| `tests/common/mod.rs`      | Shared test helpers: ephemeral NATS server, MinIO harness, `ManifestPutCrash` transport injection |

## Configuration

### StoreConfig (bucket creation only)

Config applies only at creation. If the bucket already exists, the existing one is returned as-is — `max_bytes`, `num_replicas`, `max_history`, `max_age` are ignored. To change settings on a live bucket, alter the underlying JetStream stream out-of-band.

| Field          | Default    | Rationale                                                          |
| -------------- | ---------- | ------------------------------------------------------------------ |
| `max_bytes`    | 10 MiB     | Required by Synadia Cloud; omit only for self-hosted NATS          |
| `max_history`  | 1          | Config stores rarely need change history                           |
| `num_replicas` | 1          | Set to 3 for production HA clusters                                |
| `max_age`      | None       | Set to gate retention window (also determines when cursors expire) |

### NatsConnectionConfig

| Field        | Notes                                                               |
| ------------ | ------------------------------------------------------------------- |
| `url`        | `nats://` or `tls://`; may embed `user:pass@` for legacy auth       |
| `creds`      | Base64-encoded `.creds` content (containers, ECS — no file mount)  |
| `creds_file` | Path to `.creds` on disk (bare-metal, local dev)                    |

Credentials priority: `creds` > `creds_file` > URL-embedded > no auth.

### Snapshot Tuning

| Parameter           | Effect                                                                                      |
| ------------------- | ------------------------------------------------------------------------------------------- |
| `compact_threshold` | Bytes appended since last compaction before `checkpoint()` returns `true`. Typical: 1–10 MB |

### BatchConfig (watch_applied)

| Field    | Default | Effect                                                                     |
| -------- | ------- | -------------------------------------------------------------------------- |
| `window` | 10 ms   | Max time a batch stays open before flush                                   |
| `max`    | 100     | Max updates per batch before early flush                                   |
| `channel_capacity` | 256 | Depth of the watch-task → loop channel. A full channel backpressures the watch task (by design); during state-sync hydration of a large bucket it can become the effective batch boundary, so tune it together with `max` |

| Constant | Value | Effect |
|---|---|---|
| `MAX_STORE_APPLY_FAILURES` | 16 | Consecutive transient store-apply failures before the watch fail-stops. Prevents the re-queued batch backlog from growing unboundedly under a persistently failing store. |
| `FLOOR_GUARD_INTERVAL` | 30 s | Period of the no-traffic backstop probe in the live floor guard. Primary detection is in-band (gapped delivery); this only fires when no deliveries arrive. |

### ObjectStoreTransport / ExportLease

| Setting                              | Default | Effect                                                                              |
| ------------------------------------ | ------- | ----------------------------------------------------------------------------------- |
| `prefix`                             | —       | Object-store key namespace; all payloads + pointer live under this prefix           |
| `allow_non_atomic_pointer`           | false   | Opt-in for `file://` stores that lack CAS (dev/test only); FAILS CLOSED by default  |
| `ExportLease::ttl`                   | —       | Round period; embedded in `expires_at`; determines prune grace (`4 × ttl`)          |

**Prune grace is `4 × lease_ttl`:** protects payloads a concurrent publisher may still reference (one TTL) plus in-flight bootstrap readers (three more TTLs of headroom). Increase if bootstrap download latency approaches `ttl`.

### Per-backend durability

| Backend          | Config     | Default    | Effect                                                                 |
| ---------------- | ---------- | ---------- | ---------------------------------------------------------------------- |
| `AppendLogSnapshot` | —       | page-cache | `checkpoint()` flushes to OS page cache; `compact()` fsyncs the new file |
| `FjallSnapshot`  | `sync`     | false      | `true` → fsync on every `apply` batch; `false` → OS page cache         |
| `RocksDbSnapshot` | `sync`    | false      | `true` → fsync on every `apply` WriteBatch; WAL always on              |
