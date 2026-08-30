# PedraDB

A Rust LSM-tree storage engine with a drop-in RocksDB-compatible
API (`rocksdb-compat`), and `fdatasync` before every `Ok`.

The engine is written for people who ship databases: WAL + memtable + leveled
SSTs, physical column families over one WAL, MVCC snapshots, transactions
(2PL `TransactionDB`), backup / PITR, io_uring I/O on Linux with a POSIX
fallback elsewhere, and a deterministic simulation test framework.

## Crates

| Crate | What it is |
|---|---|
| `rocksdb-compat` | The rust-rocksdb 0.22 API surface on the PedraDB engine. Swap your `rocksdb` dependency for this. |
| `pedradb-core` | The storage engine itself. |
| `pedradb-posix` | The only `unsafe` in the tree: fdatasync/fadvise. Core is `forbid(unsafe_code)`. |
| `pedradb-io-uring` | Linux io_uring `Env` (WAL/SST write + fsync); POSIX fallback. |
| `pedradb-ops` | Backup, PITR restore, format migration. |
| `pedradb-sim` | FoundationDB-style deterministic simulation testing. |

## Quickstart (drop-in swap)

```toml
[dependencies]
rocksdb-compat = "0.1.0"
```

```rust
use rocksdb_compat::{DB, Options};

let db = DB::open_default("/tmp/pedra")?;
db.put(b"k1", b"v1")?;           // fdatasync'd before Ok
let v = db.get(b"k1")?;          // Some("v1")
db.delete(b"k1")?;
```

Existing rust-rocksdb code compiles by renaming the dependency — `DB`,
`Options`, column families, `WriteBatch`(+`WithIndex`), iterators, snapshots,
`TransactionDB` / `OptimisticTransactionDB`, `Checkpoint`, `BackupEngine`,
`SstFileWriter`, `ingest_external_file`, compaction filters. Knobs that would
make the engine silently wrong (skip-any-WAL, drop-files-without-tombstones)
are refused instead of honored.

## Performance, stated precisely

All numbers are vs **RocksDB default configuration** with
`WriteOptions.sync=false` (the configuration people actually run), on a
4-vCPU Linux guest, median of 3 rounds, 2000-op zipf workload, 17 shapes
covering YCSB A–F, kvrocks, MyRocks-class, and dependency-workload patterns.

- **Async WAL column** (Pedra 64 KiB buffered WAL writes, no fdatasync):
  **12 of 17 shapes ≥ 3×**, all 17 shapes ≥ 1.0×.
- **Durability column** (Pedra `fdatasync`es before returning `Ok`, the peer
  does not): reads 1.13–1.99×, and single-client write-per-operation shapes
  run below 1× by construction — one full barrier per op against the peer's
  zero. Group commit closes that gap under concurrency (apply-shaped
  workload, 4 clients: 2.79×).
- Per-shape tables, methodology, and the below-1.0 rows are published with
  every release. Nothing is hidden; a claim you cannot audit is not a claim.

Comparison against RocksDB with `sync=true` is a different (equal-durability)
class and is never quoted as a win.

## Status

Pre-1.0 (`0.1.0`). The API surface follows rust-rocksdb 0.22 names; breaking
changes are possible until 1.0.

## License

Dual-licensed under the MIT license or the Apache License 2.0, at your
option (`LICENSE-MIT` / `LICENSE-APACHE`). This project is not affiliated
with, endorsed by, or derived from the RocksDB source; it is an independent
implementation of compatible concepts and APIs. "RocksDB" is a trademark
of its owners.
