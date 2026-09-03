<h1 align="center">PedraDB</h1>

<p align="center">
  <b>An embeddable key-value store with multi-key ACID transactions at its core.</b><br>
  Pure Rust. Fail-closed by design. Drop-in for <code>rust-rocksdb</code>.
</p>

<p align="center">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="license: MIT OR Apache-2.0"></a>
  <img src="https://img.shields.io/badge/rust-1.88%2B-orange.svg" alt="MSRV 1.88">
  <img src="https://img.shields.io/badge/status-alpha-yellow.svg" alt="status: alpha">
  <img src="https://img.shields.io/badge/unsafe-forbidden%20in%20the%20engine-success.svg" alt="unsafe forbidden in the engine">
</p>

PedraDB is a storage engine you link into your process. Keys and values are
arbitrary bytes, kept in sorted order on local disk by an LSM tree. The unit
of work is a transaction: `begin`, write the row *and* its index, `commit`.
That commit is one WAL record, fsynced before `Ok` returns.

It is built for people who put a database, a state machine, or a replicated
log on top of a key-value store, and who would rather not rebuild
consistency above the engine. It ships with `rocksdb-compat`, a
reimplementation of the rust-rocksdb 0.22 API on the same engine, so
Rocks-shaped code can swap engines by renaming one dependency.

## Why PedraDB

- **Transactions are the API, not a wrapper.** The native handle is
  transactional. In RocksDB the default handle is a `WriteBatch` store and
  transactions live in a separate `TransactionDB`; here `Db::begin()` is the
  front door, and the compat crate still offers `TransactionDB` /
  `OptimisticTransactionDB` for code that expects them.
- **Pure Rust, no C++ toolchain.** No cmake, no bindgen, no
  `librocksdb-sys` build step. 73 packages in the lockfile. The engine is
  `#![forbid(unsafe_code)]`; the only `unsafe` in the tree is two thin
  syscall crates (fdatasync / fallocate / fadvise, and io_uring submission).
- **Fail-closed, never silently wrong.** A CRC mismatch refuses the read.
  Repeated corruption refuses the open. A failed WAL sync fences the writer
  and reports the uncertain sequence range instead of continuing. Knobs that
  would turn integrity checks off are refused, not honored.
- **Machine-checked where it counts.** 21 decision kernels in the shipped
  crates (WAL recovery, manifest recovery, CRC fate, group commit, flush and
  compaction decisions, leveling, bloom filters, MVCC visibility, iterator
  windows) have Verus-verified twins: 39 proof pairs in all. Around them:
  seeded fault injection and close to 1,000 tests.
- **Drop-in for rust-rocksdb 0.22.** `DB`, column families, `WriteBatch`,
  iterators, snapshots, transactions, `Checkpoint`, `BackupEngine`,
  `SstFileWriter`, merge operators, compaction filters. Every `set_*` knob
  is classified in a machine-readable inventory. Code that reaches past the
  covered surface fails to compile; nothing is silently stubbed.
- **A modern write path.** io_uring on Linux with transparent POSIX
  fallback, group commit, a value log for large values, LZ4 block
  compression, bloom filters, block cache, sorted bulk ingest, and local
  backup with point-in-time restore.

> **Status: alpha, pre-1.0.** The on-disk format and the compat surface can
> still break. PedraDB does not have RocksDB's years of production exposure,
> and its files are not RocksDB's. Use it if you want the contracts on this
> page and can tolerate format change. Treat the benchmark numbers as lab
> measurements, not an SLA.

## Quickstart

The crates are not on crates.io yet; depend on the git repository.

### Native API

```toml
[dependencies]
pedradb-core = { git = "https://github.com/paulocsanz/pedradb" }
```

```rust
use pedradb_core::Db;

let mut db = Db::open("/tmp/pedra")?;

let mut tx = db.begin();
tx.put(b"user/42", br#"{"name":"ada"}"#)?;   // the row
tx.put(b"idx/name/ada", b"42")?;             // its secondary index
tx.commit()?;                                 // one WAL record, fsynced before Ok

let id = db.get(b"idx/name/ada");             // Some(b"42")
```

A row and its index land together or not at all. The full example, including
the abort path, is in `examples/secondary_index.rs`:

```sh
cargo run --example secondary_index -p pedradb-core
```

### Drop-in swap for rust-rocksdb

Rename the dependency; your `use rocksdb::…` lines keep compiling.

```toml
[dependencies]
# rocksdb = "0.22"
rocksdb = { git = "https://github.com/paulocsanz/pedradb", package = "rocksdb-compat" }
```

```rust
use rocksdb::{DB, Options};

let mut opts = Options::default();
opts.create_if_missing(true);
// opts.set_sync(true);            // opt into a WAL barrier before every Ok

let db = DB::open(&opts, "/tmp/pedra-compat")?;
db.put(b"k1", b"v1")?;             // RocksDB's factory WAL class: async, no per-write barrier
let v = db.get(b"k1")?;            // Some(b"v1")
db.delete(b"k1")?;
```

No C++ toolchain is needed for either path.

## PedraDB vs RocksDB

|  | PedraDB | RocksDB via `rust-rocksdb` |
|---|---|---|
| Build | Pure Rust, no C++ toolchain | C++ core; `librocksdb-sys` builds it with cc/cmake |
| Multi-key transactions | Native handle: `begin` → `commit` | Separate `TransactionDB` / `OptimisticTransactionDB` |
| Durability default | Native engine: fsync before `Ok`. Compat crate: Rocks factory (async) | Async (`sync=false`) |
| Turning checksums off | Refused (`ErrorKind::NotSupported`) | Honored |
| Failed WAL sync | Writer fenced; `resume()` reports the uncertain sequence range | Background error; `Resume()` |
| Memory safety | `#![forbid(unsafe_code)]` in the engine | C++ |
| Formal verification | 21 Verus-checked decision kernels | No |
| Production track record | Alpha. None yet | 10+ years at scale |
| Language bindings | Rust | C, C++, Java, Python, Go, and more |
| On-disk format | Own format; may change before 1.0 | Stable and widely tooled |

The last three rows are why PedraDB is alpha and RocksDB is the default. The
rows above them are why PedraDB exists.

## What's inside

- **LSM tree**: write-ahead log, memtable, leveled SSTs, bloom filters,
  table and block caches, LZ4 block compression.
- **Transactions**: multi-key ACID on the native handle, optimistic
  multi-writer transactions, MVCC snapshots with a bounded local history
  tier, snapshot reads (`get_at`, `range_at`).
- **Column families** encoded into one WAL, so a batch across families is
  one atomic record.
- **Value log** for large values (key-value separation), with its own GC.
- **Change feed** for CDC and watch layers.
- **Integrity**: CRC on every block and record, an at-rest scrub, a
  corruption journal with an escalation policy, fail-closed recovery.
- **Ops** (`pedradb-ops`): local backup, WAL shipping, point-in-time
  restore, format migration.
- **Bulk load**: sorted ingest that bypasses WAL and memtable for an
  append-only family, the same class as Rocks's `disableWAL` during load.
- **I/O**: io_uring on Linux; POSIX everywhere else; `F_FULLFSYNC` on macOS
  by default.

## Durability, stated precisely

- The **native engine (`pedradb-core`) defaults to durable**: every commit
  fsyncs the WAL before returning `Ok`. On macOS the barrier is
  `F_FULLFSYNC`-class by default.
- The **compat crate defaults to RocksDB's factory WAL class**: async, no
  barrier per write, because that is the configuration people run.
  `Options::set_sync(true)` or per-write `WriteOptions::set_sync` turns on
  the full barrier before `Ok`.
- Either way, a crash mid-write never surfaces a partial write: torn WAL
  tails recover as a clean prefix, and repeated corruption refuses the open
  rather than serving wrong data.
- A **failed WAL sync fences the writer** (`ErrorKind::Fenced`). `resume()`
  reports the uncertain sequence range and recovers explicitly.

## Compatibility in detail

`rocksdb-compat` covers the rust-rocksdb 0.22 surface most code uses: `DB`,
`Options`, column families, `WriteBatch` and `WriteBatchWithIndex`,
iterators (`iterator_cf`, `prefix_iterator`, raw), snapshots, `multi_get`,
`get_pinned`, `key_may_exist`, `delete_range`, `TransactionDB` (2PL) and
`OptimisticTransactionDB`, `Checkpoint`, `BackupEngine`, `SstFileWriter`,
`ingest_external_file`, merge operators, compaction filters, properties, and
live-file listing. Anything outside it, `rocksdb::ffi` and `librocksdb-sys`
types included, is a compile error.

Every `set_*` knob the crate exposes is a row in `KNOB_INVENTORY`, classified
as **wired** (changes engine state), **inert** (accepted for compile
compatibility, named as a no-op so you can tell), **refused** (returning `Ok`
would be silently wrong), or **safer-divergent** (behaviour differs from
Rocks on purpose and is stricter). Two examples of refused knobs:
`set_verify_checksums(false)` and skip-any-WAL recovery.

Found a rust-rocksdb behaviour this crate does not match? Open an issue. The
four filed so far each got a regression test in
`crates/rocksdb-compat/tests/repro_issues.rs` and a fix the same day.

## Benchmarks

The peer is **RocksDB default** (`WriteOptions.sync=false`), the class
production Rocks runs. Linux, single guest. Ratio > 1 means PedraDB is
faster. A win against `sync=true` would not count. macOS / APFS numbers are
not the claim. Host noise on the 25M hydrate is about 3 s, so one lucky
1.01× is not published as a win.

**Drop-in, same WAL class as production Rocks.** `rocksdb-compat` default
(WAL `write()`, no per-op barrier) vs Rocks `sync=false`. Linux 4 vCPU
(Threadripper PRO 3975WX, 2026-08-25, 3 rounds, 17/17 shapes **min > 1.0**):

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

The floor is `deps_raftlog` at 1.014. That is parity, not 2×.

**With fdatasync before `Ok`** (native default, or `set_sync(true)`) against
that same async peer: reads stay ahead (1.13–1.99× on the smoke-scale G1
battery) with the barrier on the write path. Single-client write-per-op
shapes lose by construction, one full barrier per op against the peer's
zero. Group commit closes them under concurrency (`apply_mc4` 2.79×). The
1-client write rows are the price of the contract, not wins.

**Sorted ingest** (Linux guest, 2026-09-02; 1024-op batches, ~200 B values;
latched bulk ingest skips WAL and memtable on the append-only family):

| n | hydrate | settle | get_hit | prefix_scan | get_loop | multi_get |
|---|---:|---:|---:|---:|---:|---:|
| 1M | **1.82×** | **2.50×** | **1.44×** | **1.64×** | **1.36×** | **1.08×** |
| 10M | **1.03×** | **7.67×** | 1.00× (tie) | **1.31×** | **1.07×** | **1.02×** |
| 25M | ~1.0× | **~7×** | ~1.01–1.15× | ~1.12–1.16× | 0.88–0.94× | 0.81–1.07× |

- 10M `get_hit` is a tie (confidence intervals overlap), not a win.
- 25M hydrate sits inside Rocks's 27.9–30.6 s band (PedraDB floor
  28.1–28.8 s); a 3-run median of 0.997× is not a win. `lookup_100` at 25M
  is not ≥ 1×. Settle always wins.
- Absent-key `probe_miss` can lose (bulk files ship an always-true bloom)
  and is not in the required set.
- 100M on the 3.9 GiB guest runs out of memory (RSS climbs with the open
  tail). That is a RAM bound, not a benchmark result.

## How it's tested

- **Close to 1,000 tests** across the six crates: unit tests, model tests
  against `stateright` specifications (recovery, bloom, changelog, prefix,
  range, scan), codec fuzz smoke tests, a WAL durability adversarial suite,
  and a concurrent race stress suite.
- **Seeded fault injection** (`pedradb-sim`) through a swappable `Env` seam:
  I/O errors on the Nth operation, lying fsync, short writes, torn WAL
  tails, process kill after commit. Same seed, same execution. It is a
  reproducible injection surface over the real recovery path, not a
  whole-system simulator.
- **Verus twins**: 39 kernel-to-proof pairs over 21 kernels, in 26 proof
  files in the shipped crates, checked against a pinned Verus release with
  `scripts/formal/verus_check.sh --all`. The production kernel is the source
  of record and the twin proves its decision logic. Not proven: the
  operating system, the disk, rustc, or Verus and Z3 themselves.
- **Oracle testing**: in our lab harness, workloads are diffed against real
  RocksDB. The oracle crate is not part of this repository, and RocksDB is
  never linked into the engine.

## Crates

| Crate | What it is |
|---|---|
| `pedradb-core` | The storage engine. `#![forbid(unsafe_code)]`. |
| `rocksdb-compat` | The rust-rocksdb 0.22 API surface on the engine. Swap your `rocksdb` dependency for this. |
| `pedradb-ops` | Local backup, WAL shipping, point-in-time restore, format migration. |
| `pedradb-sim` | Seeded fault injection for recovery testing. |
| `pedradb-io-uring` | Linux io_uring `Env` for WAL/SST writes and fsync; POSIX fallback elsewhere. |
| `pedradb-posix` | fdatasync / fallocate / fadvise. With `pedradb-io-uring`, the only `unsafe` in the tree. |

MSRV 1.88. Dual-licensed MIT or Apache-2.0.

## License

Dual-licensed under the MIT license or the Apache License 2.0, at your
option (`LICENSE-MIT` / `LICENSE-APACHE`). This project is not affiliated
with, endorsed by, or derived from the RocksDB source; it is an independent
implementation of compatible concepts and APIs. "RocksDB" is a trademark
of its owners.
