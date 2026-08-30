# PedraDB

**A pure-Rust LSM-tree storage engine with a drop-in RocksDB-compatible API.**

[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
![MSRV](https://img.shields.io/badge/rust-1.88%2B-orange.svg)

Swap your `rocksdb` dependency for `rocksdb-compat` and your rust-rocksdb 0.22
code compiles unchanged — no C++, no cmake, no FFI. Underneath is an engine
written for people who ship databases: WAL + memtable + leveled SSTs,
physical column families over one WAL, MVCC snapshots, transactions,
backup / PITR, io_uring I/O on Linux, and a deterministic simulation test
framework.

## Quickstart: the drop-in swap

```toml
[dependencies]
rocksdb-compat = "0.1.0"
```

```rust
use rocksdb_compat::{DB, Options};

let db = DB::open_default("/tmp/pedra")?;
db.put(b"k1", b"v1")?;           // RocksDB's default WAL class (async)
let v = db.get(b"k1")?;          // Some("v1")
db.delete(b"k1")?;
```

What compiles by renaming the dependency: `DB`, `Options`, column families,
`WriteBatch` (+`WithIndex`), iterators, snapshots, `TransactionDB` (2PL) /
`OptimisticTransactionDB`, `Checkpoint`, `BackupEngine`, `SstFileWriter`,
`ingest_external_file`, compaction filters.

Two deliberate differences from the original:

- **Knobs that would make the engine silently wrong are refused, not
  honored** — skip-any-WAL recovery, drop-files-without-tombstones.
- **A failed WAL sync fences the writer** (`ErrorKind::Fenced`) instead of
  silently continuing; `DB::resume()` reports the uncertain sequence range
  and recovers explicitly.

## Durability, stated precisely

- The **drop-in defaults to RocksDB's factory WAL class**: async, no barrier
  per write — the configuration people actually run, and the honest baseline
  for the benchmark numbers below. `options.set_sync(true)` (or per-write
  `WriteOptions.sync`) turns on the full barrier before `Ok` returns.
- The **native engine (`pedradb-core`) defaults to durable**: every commit
  fsyncs the WAL before returning `Ok`. On macOS the barrier is
  `F_FULLFSYNC`-class by default — stronger than what the `librocksdb-sys`
  crate build gives you there.
- Either way, a crash mid-write never surfaces a partial write: torn WAL
  tails recover as a clean prefix, and repeated corruption refuses the open
  rather than serving silently wrong data.

## The native API: multi-key ACID

The engine is usable directly, without the compat shim — and its own API is
transactional at the core:

```rust
use pedradb_core::Db;

let mut db = Db::open("/tmp/pedra-native")?;

let mut tx = db.begin();
tx.put(b"user/42", br#"{"name":"ada"}"#)?;   // row
tx.put(b"idx/name/ada", b"42")?;             // secondary index
tx.commit()?;                                 // one WAL record, fsynced before Ok
```

Update a row and its index in one atomic, durable commit — the thing every
system built on RocksDB had to reinvent on top.

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

## How it's tested

- **Deterministic simulation** (`pedradb-sim`): FoundationDB-style seeded
  runs — fault injection through a swappable `Env` seam: lying fsync, short
  writes, torn WAL tails, injected I/O errors. Same seed, same execution.
- **Oracle testing**: workloads are diffed against the real RocksDB as an
  external test oracle. It is never linked into the engine — only into the
  test harness.
- **Fail-closed by construction**: CRC failures, corruption, and fsync
  errors stop the engine or fence the writer; "silently wrong" is treated as
  the worst possible outcome, worse than unavailable.

## Crates

| Crate | What it is |
|---|---|
| `rocksdb-compat` | The rust-rocksdb 0.22 API surface on the PedraDB engine. Swap your `rocksdb` dependency for this. |
| `pedradb-core` | The storage engine itself. `#![forbid(unsafe_code)]`. |
| `pedradb-posix` | The only `unsafe` in the tree: fdatasync/fadvise. |
| `pedradb-io-uring` | Linux io_uring `Env` (WAL/SST write + fsync); POSIX fallback elsewhere. |
| `pedradb-ops` | Backup, PITR restore, format migration. |
| `pedradb-sim` | Deterministic simulation testing framework. |

## Status

Pre-1.0 (`0.1.0`). The API surface follows rust-rocksdb 0.22 names; breaking
changes are possible until 1.0. MSRV 1.88.

## License

Dual-licensed under the MIT license or the Apache License 2.0, at your
option (`LICENSE-MIT` / `LICENSE-APACHE`). This project is not affiliated
with, endorsed by, or derived from the RocksDB source; it is an independent
implementation of compatible concepts and APIs. "RocksDB" is a trademark
of its owners.
