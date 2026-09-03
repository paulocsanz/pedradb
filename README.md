# PedraDB

**An embeddable, persistent key-value store with multi-key ACID transactions.**

[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
![MSRV](https://img.shields.io/badge/rust-1.88%2B-orange.svg)

PedraDB is a library you link into your process. It stores keys and values
as arbitrary bytes, in sorted order, on local disk. Every multi-key update
is one ACID transaction: `commit` does not return `Ok` until that batch is
on disk. Use it as the storage kernel under a database, a state machine, or
an application that needs “update the row *and* its index” without standing
up a cluster.

RocksDB is the usual library for this job. It is fast and has no real
multi-key transactions — CockroachDB, TiKV, and the rest reinvented
consistency on top. PedraDB puts that in the core. A rust-rocksdb-compatible
crate is included so existing Rocks-shaped code can swap the engine; the
native API is `open → begin → get / put / delete / range → commit`.

> **Alpha (pre-1.0).** Lab-mature, not a production default. The on-disk
> format and the rust-rocksdb-compatible surface can still break. PedraDB
> does not have RocksDB / Pebble / FoundationDB years of field exposure.
> Use it if you want the contracts below and can tolerate format change.
> Do not treat the numbers as an SLA.

## Quickstart: the drop-in swap

`rocksdb-compat` is the rust-rocksdb 0.22 API on this engine: no C++ toolchain,
no cmake, no FFI. Code that stays on the covered surface swaps by renaming
one dependency; code that reaches into `rocksdb::ffi` fails to compile
rather than misbehave.

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
`ingest_external_file`, compaction filters. Anything outside that surface —
`rocksdb::ffi`, `librocksdb-sys` types — is a compile error, never a silent
stub.

Two deliberate differences from the original:

- **Knobs that would make the engine silently wrong are refused, not
  honored** — skip-any-WAL recovery, drop-files-without-tombstones.
- **A failed WAL sync fences the writer** (`ErrorKind::Fenced`) instead of
  silently continuing; `DB::resume()` reports the uncertain sequence range
  and recovers explicitly.

## Durability, stated precisely

- The **drop-in defaults to RocksDB's factory WAL class**: async, no barrier
  per write — the configuration people actually run. `options.set_sync(true)`
  (or per-write `WriteOptions.sync`) turns on the full barrier before `Ok`
  returns.
- The **native engine (`pedradb-core`) defaults to durable**: every commit
  fsyncs the WAL before returning `Ok`. On macOS the barrier is
  `F_FULLFSYNC`-class by default — stronger than what the `librocksdb-sys`
  crate build gives you there.
- Either way, a crash mid-write never surfaces a partial write: torn WAL
  tails recover as a clean prefix, and repeated corruption refuses the open
  rather than serving silently wrong data.

## Benchmarks

The only peer that counts as beating Rocks is **RocksDB default**
(`WriteOptions.sync=false`). Pedra still `fdatasync`s before `Ok` when you
opt into that (native default / `set_sync(true)`). A win against
`sync=true` is not a win. macOS / APFS numbers are not the claim.

Lab, Linux, single guest. Ratio > 1 means Pedra is faster. Host noise on
the 25M hydrate is ~3 s / ~10 % of Pedra; one lucky 1.01× is not a
published win.

### Drop-in — same WAL class as the Rocks people run

`rocksdb-compat` default (`PEDRA_PARITY_ASYNC=1`: WAL `write()`, no
per-op barrier) vs Rocks `sync=false`. This is engine speed at the
durability class production Rocks actually uses. It is the regression
gate, not “more durable *and* faster.”

Linux 4 vCPU (Threadripper PRO 3975WX, 2026-08-25, 3 rounds, 17/17
shapes **min > 1.0**):

| shape | median × | min × |
|---|---:|---:|
| ycsb_a | 2.24 | 1.45 |
| ycsb_b | 2.26 | 1.46 |
| ycsb_c | 3.40 | 2.21 |
| ycsb_d | 3.03 | 1.71 |
| ycsb_e | 12.6 | 7.18 |
| ycsb_f | 2.30 | 1.07 |
| deps_raftlog (tightest) | 1.24 | **1.014** |
| kvrocks_set | 2.58 | 1.98 |
| kvrocks_get | 4.75 | 4.05 |

The floor is `deps_raftlog` 1.014. That is parity, not 2×.

### Stronger durability — fdatasync before Ok vs that same peer

Reads stay ahead (1.13–1.99× on the smoke-scale G1 battery) **with** the
barrier on the write path. Single-client write-per-op shapes lose by
construction: one full barrier per op vs the peer’s zero. Group commit
closes them under concurrency (`apply_mc4` 2.79×). Those 1-client write
rows are not wins; they are the price of the contract. Never hidden,
never quoted as beating Rocks.

### Sorted ingest (slipstream hydrate) — Linux guest, 2026-09-02

Same peer. 1024-op batches, ~200 B values. Pedra latched bulk ingest
skips WAL + memtable on the append-only family (same class as Rocks
`disableWAL` during load; process crash loses the uninstalled tail).

| n | hydrate | settle | get_hit | prefix_scan | get_loop | multi_get |
|---|---:|---:|---:|---:|---:|---:|
| 1M | **1.82×** | **2.50×** | **1.44×** | **1.64×** | **1.36×** | **1.08×** |
| 10M | **1.03×** | **7.67×** | 1.00× (tie) | **1.31×** | **1.07×** | **1.02×** |
| 25M | ~1.0× | **~7×** | ~1.01–1.15× | ~1.12–1.16× | 0.88–0.94× | 0.81–1.07× |

- **1M:** every required leg > 1×.
- **10M:** `get_hit` is a tie (confidence intervals overlap). Not counted
  as a win.
- **25M:** hydrate sits in Rocks’s 27.9–30.6 s band (Pedra floor
  28.1–28.8 s); a 3-run median of 0.997× is not a published win.
  `lookup_100` is not ≥ 1×. Settle always wins.
- Absent-key `probe_miss` can lose (bulk files ship an always-true bloom)
  and is **not** in the required set.

100M on the 3.9 GiB guest OOMs today (RSS climbs with the open tail).
That is a RAM bound, not a “we lost the bench.”

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

**Alpha**, pre-1.0 (`0.1.0`). The API surface follows rust-rocksdb 0.22
names; breaking changes are possible until 1.0, including the on-disk
format. MSRV 1.88. This is not a substitute for years of production
burn-in, and the files on disk are not RocksDB’s.

## License

Dual-licensed under the MIT license or the Apache License 2.0, at your
option (`LICENSE-MIT` / `LICENSE-APACHE`). This project is not affiliated
with, endorsed by, or derived from the RocksDB source; it is an independent
implementation of compatible concepts and APIs. "RocksDB" is a trademark
of its owners.
