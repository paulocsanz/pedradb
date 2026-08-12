# PedraDB usage (P0)

**Status:** user-facing minimal docs for justify-use  
**Updated:** 2026-08-11  
**API:** `pedradb-core` — `Db`, `Transaction`, `OpenOptions`

---

## One-liner

Embedded ordered key-value store with **multi-key ACID** in one process.

```text
open → begin → get / put / delete → commit
```

No server. No multi-node. No SQL in core. Link the crate into your app.

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

## Durability (read this)

| Call | Default (`OpenOptions { sync: true }`) |
|------|----------------------------------------|
| `put` / `delete` / `tx.commit()` returns `Ok` | WAL appended **and** synced to disk |
| Process kill after `Ok` | Committed data recoverable on `open` |
| Crash mid-write | Truncated tail skipped; **no partial multi-key TX** |
| Uncommitted TX drop | Nothing durable |
| `OpenOptions { sync: false }` | Faster; **may lose acked writes on power loss** — benches/bulk only |

Full contract: rustdoc on `db` module.

---

## API surface (P0)

| API | Notes |
|-----|--------|
| `Db::open(path)` | Directory; creates if missing |
| `Db::open_with(path, OpenOptions)` | `sync` flag |
| `Db::get` / `put` / `delete` | Auto-commit |
| `Db::begin` → `Transaction` | Exclusive `&mut Db` (single-writer) |
| `tx.get` / `put` / `delete` | Staging + snapshot reads |
| `tx.commit` / `tx.abort` | Multi-key atomic / discard |
| `Db::flush` | MemTable → new `.sst`, rotate WAL (P1.1) |
| `Db::range(start, end)` | Ordered scan MemTable ∪ SSTs at latest snapshot (P1.2) |
| `Db::compact` / `compact_with` | Merge SSTs (tmp→rename); optional version GC |
| `WriteOptions` / `put_with` / `apply_batch_with` | Per-write sync or `no_sync` + later `Db::sync` (group fsync) |
| `OpenOptions.auto_flush_bytes` | Auto SST flush when MemTable grows (default 4 MiB) |
| `OpenOptions.auto_compact_sst_count` | After flush, compact when SST count ≥ N (`None` = off) |
| `OpenOptions.exclusive` | Default `true`: PID `LOCK` file (cross-process; same-PID re-open steals) |
| `CompactOptions` / `CompactGcOptions` | `latest_only` or `min_sequence` watermark during compact |
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

**Not in P0:** `range` on public API (MemTable has it; SST/merge later), multi-process open, network, SQL.

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
