# PedraDB usage (P0)

**Status:** user-facing minimal docs for justify-use  
**Updated:** 2026-08-12  
**API:** `pedradb-core` — `Db`, `Transaction`, `OpenOptions`  
**Engine maturity:** [RFC-0014](rfc/0014-rocks-pebble-redwood-maturity.md)

---

## One-liner

Embedded ordered key-value store with **multi-key ACID** in one process.

```text
open → begin → get / put / delete → commit
```

No server. No multi-node. No SQL in core. Link the crate into your app.

A future **`pedra-map`** crate may expose sled/BTreeMap-shaped helpers (`insert`,
`open_tree`, CAS) as a **layer** on this API — same storage, no second engine.
See [`performance-ceiling-option-preservation-and-sled-layer.md`](performance-ceiling-option-preservation-and-sled-layer.md).

---

## Quick start

```toml
# Cargo.toml
pedradb-core = { path = "crates/pedradb-core" }  # or version once published
```

```rust
use pedradb_core::Db;

fn main() -> pedradb_core::Result<()> {
    let mut db = Db::open("/tmp/pedra-demo")?;

    // Auto-commit single key
    db.put(b"hello", b"world")?;
    assert_eq!(db.get(b"hello").as_deref(), Some(b"world".as_ref()));

    // Multi-key transaction (row + secondary index)
    {
        let mut tx = db.begin();
        tx.put(b"u/42", br#"{"name":"ada"}"#)?;
        tx.put(b"idx/name/ada", b"42")?;
        tx.commit()?; // one WAL record, fsync by default
    }

    assert_eq!(db.get(b"u/42").as_deref(), Some(br#"{"name":"ada"}"#.as_ref()));
    assert_eq!(db.get(b"idx/name/ada").as_deref(), Some(b"42".as_ref()));

    db.close()?;
    Ok(())
}
```

Reopen the same directory after process exit — committed keys are still there (WAL replay).

---

## CAS, sequence pin, change feed (RFC-0019)

These are the L1 hooks for leases/watch/CDC layers (Scylla-need without DIY RMW).

```rust
use pedradb_core::{Db, ScanProjection, Snapshot};
use std::ops::Bound;

fn layer_sketch(db: &mut Db) -> pedradb_core::Result<()> {
    // Conditional put (IF NOT EXISTS / version CAS)
    let seq = db.put_if_absent(b"lease/vol-1", b"holder-a")?;
    assert!(db.put_if_absent(b"lease/vol-1", b"holder-b").is_err()); // CasMismatch
    let seq2 = db.compare_and_swap(b"lease/vol-1", b"holder-a", b"holder-c")?;

    // Layer pin: after Ok, get_at(seq) sees the write; get_at(seq-1) does not
    assert_eq!(
        db.get_at(Snapshot::at(seq2), b"lease/vol-1").as_deref(),
        Some(b"holder-c".as_ref())
    );

    // Change feed for watch catch-up: (from, to] exclusive lower, inclusive upper
    let changes = db.changes(seq, seq2)?;
    assert!(!changes.is_empty());
    let tail = db.changes_after(seq2); // empty until next durable commit

    // multi_get + key-only scan (cheap listings)
    let _ = db.multi_get(&[b"a", b"b", b"c"]);
    for kv in db.scan_projected(Bound::Unbounded, Bound::Unbounded, ScanProjection::KeyOnly) {
        assert!(kv.value.is_empty());
        let _ = kv.key;
    }
    let _ = tail;
    Ok(())
}
```

| API | Role |
|-----|------|
| `put_if_absent` / `put_if_eq` / `compare_and_swap` | First-class CAS; fail closed on mismatch |
| `put_with` / `delete_with` / `apply_batch` / `tx.commit` | Return **commit sequence** (layer pin) |
| `changes(from, to)` / `changes_after(from)` | Post-commit logical feed (durable CHANGELOG) |
| `multi_get` / `multi_get_at` | N point reads, same visibility as `get` |
| `scan_projected(..., KeyOnly)` | Keys without loading values |

---

## Durability (read this)

| Call | Default (`OpenOptions { sync: true }`) |
|------|----------------------------------------|
| `put` / `delete` / `tx.commit()` returns `Ok` | WAL appended **and** synced to disk |
| Process kill after `Ok` | Committed data recoverable on `open` |
| Crash mid-write | Truncated tail skipped; **no partial multi-key TX** |
| Uncommitted TX drop | Nothing durable |
| `OpenOptions { sync: false }` | Faster; **may lose acked writes on power loss** — benches/bulk only |
| **`Err` on required WAL sync** | **Uncertain outcome** — append may have landed; handle is **durability-fenced** ([`CoreError::DurabilityFenced`]) until `close` + `open` |
| Further puts after fence | Always `DurabilityFenced` (no silent continue) |
| Reopen after fence | Rebuilds mem from WAL; recovered prefix is consistent (failed-sync write may appear) |
| Flush / MANIFEST / checkpoint with `sync=true` | **`Env::sync_dir` errors are propagated** (not discarded) |
| `sync=false` dir fsync | Best-effort discard still OK |

Full contract: rustdoc on `db` module. Audit fix backlog: [RFC-0015](rfc/0015-audit-pedradb-correctness-fixes.md).

---

## API surface (P0)

| API | Notes |
|-----|--------|
| `Db::open(path)` | Directory; creates if missing |
| `Db::open_with(path, OpenOptions)` | `sync` flag |
| `Db::open_with_env` / `open_with_host` | Inject `Env` / full `Host` (DST; see [dst-seams](dst-seams.md)) |
| `pedradb_io_uring::open` / `IoUringEnv` | Linux **io_uring** write+fsync Env (POSIX fallback on macOS/dev) |
| `Db::get` / `put` / `delete` | Auto-commit |
| `Db::begin` → `Transaction` | Exclusive `&mut Db` (single-writer) |
| `tx.get` / `put` / `delete` | Staging + snapshot reads |
| `tx.commit` / `tx.abort` | Multi-key atomic / discard |
| `Db::flush` | MemTable → new `.sst`, rotate WAL (P1.1); auto-compact failures do **not** fail flush (see `DbStats.auto_compact_failures`) |
| `Db::range(start, end)` | Convenience scan → materialises a `Vec` (**OOM footgun** on large DBs; small-DB/tests only) |
| `Db::range_limited(…, limit)` | **Preferred** for pagination — stop after N live keys |
| `Db::scan` / `scan_at` | **Preferred** streaming merge for large ranges (bound memory) |
| `Db::compact` / `compact_with` | Merge SSTs (tmp→rename); optional version GC |
| `Db::create_checkpoint(dest)` | Point-in-time copy (flush + file set); openable as a DB |
| `pedradb_ops::BackupEngine` | Local base backup, `ship_wal`, `restore` / `restore_pitr`, verify |
| `pedradb_ops::migrate_to_latest` / `inspect_format` | Format inspect + rewrite SSTs/MANIFEST to current writer |
| CLI `pedra backup\|restore\|pitr\|ship-wal\|migrate\|inspect` | Ops suite from the command line |
| `Db::stats()` → `DbStats` | Mem/SST/WAL sizes, cache hits, `wal_sync_count`, `vlog_*`, amp counters (`bytes_ingested` / `bytes_written_*` / `compact_count`) |
| `ConcurrentDb` | Multi-thread handle: **write group** amortizes fsync; dual-mem flush pipeline (not full Rocks multi-writer) |
| `ConcurrentDb::begin_occ` / `OccTransaction` | **OCC multi-writer** TX: conflict → `TransactionConflict` (RFC-0014 P2.1) |
| `OpenOptions.large_value_threshold` | **Opt-in** (`None` default): spill large values to `VALUES.vlog` (WiscKey-shaped) |
| `Db::compact_vlog()` | Crash-safe value-log GC rewrite (RFC-0016 P0.1); reclaim after overwrite/delete + SST version drop |
| `BackupEngine::create_incremental` / `restore_with_increments` | Incremental WAL archive + restore (RFC-0014 P2.3) |
| `Db::verify_checksums()` | Re-validate SST + WAL integrity (fail-stop on bitrot) |
| `WriteOptions` / `put_with` / `apply_batch_with` | Per-write sync or `no_sync` + later `Db::sync` (group fsync) |
| `OpenOptions.auto_flush_bytes` | Auto SST flush when MemTable grows (default 4 MiB) |
| `OpenOptions.auto_compact_sst_count` | After flush, compact when SST count ≥ N (`None` = off) |
| `OpenOptions.auto_compact_sst_bytes` | After flush, compact when total SST bytes ≥ N (`None` = off) |
| `OpenOptions.exclusive` | Default `true`: PID `LOCK` file (cross-process; same-PID re-open steals) |
| `CompactOptions` / `CompactGcOptions` | `latest_only` or `min_sequence` watermark during compact |
| SST v3 | Block layout + on-disk Bloom; v1/v2 still readable |
| MANIFEST / `CURRENT` | Live SST inventory rewritten on flush/compact; orphan SST GC on open |
| `Db::last_sequence` / `sst_count` / `path` / `sync` / `close` | Introspection / shutdown |
| `pedradb-apply` | `LogApplier`, `FakeLog`, `InProcessCluster`, `KvService` (RFC-0010) |
| `pedradb-replicate` | WAL-shipped async read replica (`WalShipper`, `bootstrap_replica_from_wal`) |
| `pedradb-raft` | In-process Raft (election + log replicate + apply to PedraDB) |
| `pedradb-dcs` | etcd-class DCS SM: CAS, leases, watch, leader lock (Patroni path) |
| `pedradb-sql` | Minimal SQL: CREATE/INSERT/SELECT/DELETE over tables-as-prefixes |
| `pedradb-stream` | Durable append stream + consumer cursors (JetStream-class *need*) |
| `pedradb-sim` | FailingEnv, RecordingEnv, lying sync, short-write, `from_seed`, FailingEnvArc |

Multi-node / wire (RFC-0012 **delivered**):

| Crate / bin | Role |
|-------------|------|
| `pedradb-raft` + `pedra-raft-node` | TCP Raft, persisted hard state/log |
| `pedradb-dcs` + Raft | `DcsCommand` proposed on leader → applied on all nodes (`propose_dcs` / `dcs_get`) |
| `pedradb-http` | HTTP KV + DCS (leader/CAS/lease) |
| `pedradb-dst` | Seedable FailingEnv sweep harness |

**DCS over Raft (e2e):** leader runs `PeerClient::propose_dcs(DcsCommand::Create{...})`; followers answer `dcs_get` with the same key after commit.

**MontanhaDb (Montan-HA-DB)** — multi-node HA product on PedraDB:  
[docs/montanhadb.md](montanhadb.md)

**Montanha-Store** (`pedradb-store`): multi-Raft ranges + **DCS on store**
(`dcs_create` / `dcs_cas` → `apply_dcs_command` on commit). Layering:
[montanha-layering-dcs-on-store.md](montanha-layering-dcs-on-store.md).

**Patroni-shaped HA + live leadership stream (design):**  
[docs/live-leadership-and-patroni-shaped-ha.md](live-leadership-and-patroni-shaped-ha.md) — two planes (truth vs best-effort), open sessions, fencing by revision, roadmap.

---

## Large values (`VALUES.vlog`) — RFC-0016

**Default is off** (`large_value_threshold: None`). Enable only when you measure a large-value write-amp win.

```rust
use pedradb_core::{Db, OpenOptions};

let mut db = Db::open_with(
    "/tmp/pedra-large",
    OpenOptions {
        large_value_threshold: Some(4 * 1024), // spill ≥ 4 KiB
        ..OpenOptions::default()
    },
)?;

// Under update/delete churn of large values, reclaim space:
// 1) Prefer compacting old SST versions first (or accept multi-version live refs).
// 2) Then rewrite the value log:
let stats = db.compact_vlog()?;
// stats.bytes_before / bytes_after / live_records
```

| Rule | Why |
|------|-----|
| Threshold **off** by default | Avoid silent disk fill in production |
| Watch `DbStats.vlog_bytes` vs `vlog_live_bytes` | When `vlog_bytes ≫ vlog_live_bytes`, call `compact_vlog` |
| Checkpoint/backup copies full `VALUES.vlog` | Correctness requires the log, including unreclaimed garbage until GC |
| GC keeps every VLG1 still referenced by mem/imm/**any SST version** | Run SST compact / `latest_only` carefully before expecting big reclaim |

**Crash safety:** GC writes `VALUES.vlog.new`, then remaps SST pointers and swings **MANIFEST** with `vlog_use_new=true` (atomic CURRENT). Only then does open prefer `.new`. Before that MANIFEST install, open keeps the primary vlog + old offsets. After MANIFEST and before promote, open uses `.new` + remapped SSTs.

---

## Launch readiness checklist (RFC-0016 P2.4)

Before calling an embed deployment “launch-ready”:

| Gate | Check |
|------|--------|
| Durability | Default `sync: true`; understand fence on failed WAL sync |
| Large values | Threshold off **or** GC scheduled + `vlog_bytes` alerted |
| Amp visibility | `DbStats` fsync / ingest / SST write counters monitored |
| Integrity | `verify_checksums` in ops path; fail-stop on CRC |
| Soak | Fixed-seed put/get/delete/flush/compact/vlog-GC with silent_wrong=0 (CI test) |
| Backup | Checkpoint or `BackupEngine` exercised under your write load (P2.1 continuous still open) |
| Encryption | **Non-goal in core** — use FS encryption (LUKS, cloud volume) or app-layer AEAD |
| Concurrency | `ConcurrentDb` group-commit + dual-mem; not Rocks multi-mem writer class |
| Honesty | Do not claim field parity with Rocks/Pebble/FDB ([robustness doc](robustness-vs-rocks-pebble-fdb.md)) |

---

## Ops hygiene (RFC-0015 P2)

### Scans (prefer limited / streaming)

```rust
use std::ops::Bound;
use pedradb_core::Db;

fn list_page(db: &Db, start: &[u8], limit: usize) {
    // Good: pagination
    let page = db.range_limited(Bound::Included(start), Bound::Unbounded, Some(limit));
    // Good: streaming (large ranges)
    for kv in db.scan(Bound::Included(start), Bound::Unbounded).take(limit) {
        let _ = kv;
    }
    // Avoid on large DBs: materialises the whole interval
    // let all = db.range(Bound::Unbounded, Bound::Unbounded);
}
```

### ConcurrentDb contention (M2)

`ConcurrentDb` serialises **writers** with a write lock that holds through WAL
`fsync`. Concurrent puts are correct and linearizable, but write QPS is not
RocksDB multi-writer class. Readers share a read lock.

### Raft integration wall sleep (M5)

`pedradb-raft` multi-process / TCP integration tests may use `thread::sleep` for
election timing. That is **OK for tests only**. The Montanha **store** control
plane uses Queued RPC + logical time (`advance_time`) — keep wall sleep out of
store DST / production control paths.

### Supply chain (M3)

Workspace root [`deny.toml`](../deny.toml) configures advisory checks. CI runs
`cargo deny check advisories` (see `.github/workflows/supply-chain.yml`).
**Owner:** maintainers run `cargo deny check advisories` (or `cargo audit`) on
release tags and dependency bumps if CI is skipped.

---

## Layer sketch: secondary index

PedraDB does **not** implement indexes. You do, with multi-key TX:

```rust
use pedradb_core::{Db, Result};

/// Toy user store: primary key `u/{id}`, index `idx/email/{email}` → id bytes.
fn upsert_user(db: &mut Db, id: &str, email: &str, payload: &[u8]) -> Result<()> {
    let pk = format!("u/{id}");
    let idx = format!("idx/email/{email}");

    let mut tx = db.begin();

    // Optional: remove old index if email changed (read old row first)
    if let Some(old) = tx.get(pk.as_bytes()) {
        // parse old email from payload in a real app; here we only set new idx
        let _ = old;
    }

    tx.put(pk.as_bytes(), payload)?;
    tx.put(idx.as_bytes(), id.as_bytes())?;
    tx.commit()
}

fn lookup_by_email(db: &Db, email: &str) -> Option<Vec<u8>> {
    let idx = format!("idx/email/{email}");
    let id = db.get(idx.as_bytes())?;
    let pk = format!("u/{}", String::from_utf8_lossy(&id));
    db.get(pk.as_bytes()).map(|b| b.to_vec())
}
```

**Rule:** any mutation that touches data **and** an index entry must use **one** `Transaction`.  
Prefixes (`u/`, `idx/`) are application convention — not a kernel feature (FDB-style subspaces).

---

## What you get vs RocksDB / fjall

| | PedraDB P0 | Typical RocksDB |
|--|------------|-----------------|
| Multi-key atomic | **Yes** (`begin`/`commit`) | WriteBatch atomicity without interactive TX by default |
| Ordered keys | Yes (MemTable; range public later) | Yes |
| Default durability | **Sync on commit** | Often async WAL |
| Embed | Pure Rust, `forbid(unsafe_code)` | C++ |

---

## Limits (honest P0)

- Data lives in **MemTable + WAL + SSTs** after `flush`; call `compact` to merge SSTs.  
- One writer at a time (`&mut` on begin / put); OCC multi-writer not in P1.  
- `range` is available; no SQL/network.  
- Not a multi-node product; see grail plan for outer rungs.

---

## See also

- [RFC-0001](rfc/0001-pedradb-high-level-spec.md) — product spec  
- [RFC-0004](rfc/0004-transaction-api.md) — transaction  
- [positioning.md](positioning.md) — why this exists  
