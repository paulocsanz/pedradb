<h1 align="center">PedraDB</h1>

<p align="center">
  <b>An embeddable key-value store with multi-key ACID transactions at its core.</b><br>
  Pure Rust. Durable by default. Fail-closed by design.
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
consistency above the engine. The API is
`open → begin → get / put / delete / range → commit`.

## Why PedraDB

- **Transactions are the API, not a wrapper.** The handle you open is
  transactional. Update a row and its secondary index in one commit, and
  either both land or neither does. Every system built on RocksDB had to
  reinvent this above the engine; here it is the engine.
- **Durable by default.** `commit` fsyncs the WAL before returning `Ok`.
  On macOS the barrier is `F_FULLFSYNC`-class by default. You can trade
  durability for speed explicitly; you never get the trade silently.
- **Fail-closed, never silently wrong.** A CRC mismatch refuses the read.
  Repeated corruption refuses the open. A failed WAL sync fences the writer
  and reports the uncertain sequence range instead of continuing. There is
  no option to turn integrity checks off.
- **Pure Rust, no C++ toolchain.** No cmake, no bindgen; the engine's
  dependency graph is 73 packages, all Rust. The engine is
  `#![forbid(unsafe_code)]`; the only `unsafe` in the tree is two thin
  syscall crates (fdatasync / fallocate / fadvise, and io_uring
  submission). One C++ exception, optional and explicit: the RocksDB
  peer behind `rocksdb-parity-bench --features real` (off by default;
  the engine never links it).
- **Machine-checked where it counts.** 21 decision kernels (WAL recovery,
  manifest recovery, CRC fate, group commit, flush and compaction decisions,
  leveling, bloom filters, MVCC visibility, iterator windows) have
  Verus-verified twins: 39 proof pairs in all. Around them: seeded fault
  injection and close to 1,000 tests.
- **A modern write path.** io_uring on Linux with transparent POSIX
  fallback, group commit, a value log for large values, LZ4 block
  compression, bloom filters, block cache, sorted bulk ingest, and local
  backup with point-in-time restore.

> **Status: alpha, pre-1.0.** The on-disk format and the API can still
> break. PedraDB does not have RocksDB's years of production exposure. Use
> it if you want the contracts on this page and can tolerate format change.
> Treat the benchmark numbers as lab measurements, not an SLA.

## Quickstart

The crates are not on crates.io yet; depend on the git repository.

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

No C++ toolchain is needed.

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

- **Every commit fsyncs the WAL before returning `Ok`.** That is the
  default (`OpenOptions::sync = true`). On macOS the barrier is
  `F_FULLFSYNC`-class by default.
- **A crash mid-write never surfaces a partial write.** Torn WAL tails
  recover as a clean prefix, and repeated corruption refuses the open
  rather than serving wrong data.
- **A failed WAL sync fences the writer** (`DurabilityFenced`).
  `fence_report()` gives the uncertain sequence range, and closing and
  reopening recovers explicitly. The engine never continues past a sync it
  cannot vouch for.
- **Async WAL is opt-in**, and the benchmarks below say which class each
  number was measured at.

## Benchmarks

The peer is **RocksDB default** (`WriteOptions.sync=false`), the class
production Rocks runs. Linux, single guest. Ratio > 1 means PedraDB is
faster. A win against `sync=true` would not count. macOS / APFS numbers are
not the claim. Host noise on the 25M hydrate is about 3 s, so one lucky
1.01× is not published as a win.

**Async WAL, same class as production Rocks.** PedraDB with WAL `write()`
and no per-op barrier vs Rocks `sync=false`. This is engine speed at equal
durability. Linux 4 vCPU (Threadripper PRO 3975WX, 2026-08-25, 3 rounds,
17/17 shapes **min > 1.0**):

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

**With fdatasync before `Ok`** (the default) against that same async peer:
reads stay ahead (1.13–1.99× on the smoke-scale G1 battery) with the
barrier on the write path. Single-client write-per-op shapes lose by
construction, one full barrier per op against the peer's zero. Group commit
closes them under concurrency (`apply_mc4` 2.79×). The 1-client write rows
are the price of the contract, not wins.

**Sorted ingest** (Linux guest; ~200 B values; latched bulk ingest skips
WAL and memtable on the append-only family). Ratio > 1 means PedraDB is
faster than RocksDB default. 25M and 100M rows are 3-run medians.

| n | hydrate | settle | get_hit | prefix_scan | get_loop | multi_get |
|---|---:|---:|---:|---:|---:|---:|
| 1M | **1.82×** | **2.50×** | **1.44×** | **1.64×** | **1.36×** | **1.08×** |
| 10M | **1.03×** | **7.67×** | 1.00× (tie) | **1.31×** | **1.07×** | **1.02×** |
| 25M | 1.02× | **27×** | **1.14×** | **1.34×** | — | **1.27×** |
| 100M | **1.23×** | **88×** | **1.80×** | 1.00× (tie) | **1.68×** | **1.74×** |

- 10M `get_hit` is a tie (confidence intervals overlap), not a win.
- 25M hydrate (3-run median 1.02×: Pedra 29.5 s vs Rocks 30.2 s) sits
  inside Rocks's 27.9–34.4 s band (Pedra floor 28.1–30.2 s) — parity
  inside the ±3 s host-noise band, not a win claim. 25M `get_loop` is
  not published: Rocks ran out of band on that lookup. Settle always
  wins.
- 100M used to OOM on the 3.9 GiB guest (sparse-index keys pinned the
  ingest key pool; fixed — index boundary keys are owned copies now).
  2026-09-04, 3 runs, same guest, full readlegs: hydrate **1.23×**
  (Pedra 123.1–124.1 s, Rocks 145–164 s; Pedra matches the 2026-09-03
  hydrate-only 3-run 121.6–127.5 s). On disk Pedra 23.97 GiB (257
  B/entry) ×3. Rocks-side spread on hydrate is still there.
- 100M reads, same 3 runs: `get_hit` **1.80×** (58 vs 104 µs),
  `get_loop` **1.68×**, `multi_get` **1.74×**. `prefix_scan` is a
  **1.00× tie** (one run 0.75×) — not a win. An earlier single-run
  printed 2.4–2.9× on the point-read cells; that was Rocks having a
  slower miss path that day (125 µs vs 104 µs here). Pedra barely
  moved. Settle **88×** (0.7 vs 61 s) is the same story in reverse:
  Rocks settle was 23 s on that single-run, 55–62 s here.
- Absent-key `probe_miss` can lose (bulk files ship an always-true bloom)
  and is not in the required set.

## How it's tested

- **Close to 1,000 tests** across the seven crates: unit tests, model tests
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

## Coming from RocksDB

`rocksdb-compat` implements the rust-rocksdb 0.22 API on this engine, so
existing Rocks-shaped Rust code can try PedraDB by renaming one dependency.
It is a migration path, not the product: it defaults to Rocks's async WAL
class so numbers compare like for like, and code that reaches past the
covered surface fails to compile rather than misbehave.

```toml
rocksdb = { git = "https://github.com/paulocsanz/pedradb", package = "rocksdb-compat" }
```

## Crates

| Crate | What it is |
|---|---|
| `pedradb-core` | The storage engine. `#![forbid(unsafe_code)]`. |
| `pedradb-ops` | Local backup, WAL shipping, point-in-time restore, format migration. |
| `pedradb-sim` | Seeded fault injection for recovery testing. |
| `pedradb-io-uring` | Linux io_uring `Env` for WAL/SST writes and fsync; POSIX fallback elsewhere. |
| `pedradb-posix` | fdatasync / fallocate / fadvise. With `pedradb-io-uring`, the only `unsafe` in the tree. |
| `rocksdb-compat` | rust-rocksdb 0.22 API on the engine, for migrating existing Rocks code. |
| `rocksdb-parity-bench` | The parity harness behind the tables above: same shape, same host, Pedra vs RocksDB default. |

MSRV 1.88. Dual-licensed MIT or Apache-2.0.

## License

Dual-licensed under the MIT license or the Apache License 2.0, at your
option (`LICENSE-MIT` / `LICENSE-APACHE`). This project is not affiliated
with, endorsed by, or derived from the RocksDB source; it is an independent
implementation of compatible concepts and APIs. "RocksDB" is a trademark
of its owners.
