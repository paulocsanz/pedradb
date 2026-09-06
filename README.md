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
`open → begin → get / put / delete → commit`.

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
- **Machine-checked where it counts.** 23 decision kernels have
  Verus-verified twins — 47 proof pairs (table in
  [Verification](#verification)). Around them: seeded fault
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
the abort path, is in `examples/secondary_index.rs`. The gallery is a ladder
from hello-world to a small ledger: [`examples/`](examples/).

```sh
cargo run -p pedradb-examples --example hello
cargo run -p pedradb-examples --example bank
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

## Verification

Not “no bugs” — machine-checked where it counts. 23 decision kernels
have Verus-verified twins: 47 proof pairs in 34 proof files in the
shipped crates, checked with `scripts/formal/verus_check.sh --all`
against a pinned Verus. The production kernel is the source of record;
the twin proves its decision logic. Not proven: the OS, the disk,
rustc, Verus, or Z3.

| Area | Proves | Proof files |
|---|---|---|
| WAL recovery | a torn tail recovers as a clean prefix; record framing | `wal_recover`, `write_record_count` |
| Manifest & reopen | newest consistent MANIFEST; reopen under WAL damage; changelog rebuild; crash-dictionary link | `manifest_recover`, `reopen_outcome`, `changelog_rebuild`, `dictionary_link` |
| CRC fate | a mismatch refuses the read — fail-closed | `sst_crc_fate`, `crc_match` |
| Durability syscalls | fdatasync and io_uring completion return codes | `fdatasync_rc`, `cqe_res` |
| Group commit | queue drain and commit visibility; wait-for is deadlock-free | `group_commit`, `wait_for_deadlock` |
| Flush & compaction | when to flush, when to compact, which CF a rewrite lands in | `flush_decision`, `compact_decision`, `compact_rewrites_sst_cf` |
| Leveling | the level-size ladder and the two-level pick | `leveling`, `leveling_pick` |
| Bloom filters | no false negatives; header bound fails closed | `bloom_filter`, `bloom_header` |
| MVCC visibility | snapshot visibility | `visible_at` |
| Iterators & scans | window keep, prefix exclusive-end, range-tombstone cover, scan guard | `iter_window`, `prefix_exclusive_end`, `range_covers`, `scan_guard` |
| Key codecs | sequence+type packing; CF family, prefix codec round-trip, SST family inference | `ikey_pack`, `cf_family`, `cf_family_of`, `cf_encode_effective`, `encode_cf_key`, `decode_cf_key`, `infer_sst_cf` |
| Probe order | point lookups probe newest-first; a newer tombstone is never shadowed | `probe_order` |
| Value log | GC decision | `vlog_gc_decision` |
| PITR restore | archived WAL record is replayed iff `base < seq ≤ target`; a future seq cannot appear | `pitr_window` |

## Benchmarks

The peer is **RocksDB default** (`WriteOptions.sync=false`), the class
production Rocks runs. Ratio > 1 means PedraDB is faster. Bold is a win;
plain is a tie or parity; `—` is not measured or refused. A win against
`sync=true` would not count. macOS / APFS numbers are not the claim.
Protocol, per-run values, and the full loss registry live in
[`docs/benchmarks.md`](docs/benchmarks.md).

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
construction, one full barrier per op against the peer's zero. Group
commit closes them under concurrency (`apply_mc4` 2.79×). The 1-client
write rows are the price of the contract, not wins.

**Sorted ingest — results.** Clustered `route.svc-*` keys, 200 B values,
1024-entry batches, 256 MiB cache, one backend per process. Harness:
`snapshot_backends`, in-tree at `crates/snapshot-bench`. Pedra row,
RocksDB row, ratio row. The 1M/10M rows and the 25M read cells are single
official runs (2026-09-02/03); the 25M hydrate and every 100M cell are
3-run medians (2026-09-03/05).

| 1M | hydrate | settle | get_hit | prefix_scan | get_loop | multi_get | probe_miss p50 | disk |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Pedra | 0.6 s | 0.4 s | 3.0 µs | 202 µs | 313 µs | 336 µs | — | 0.24 GiB |
| Rocks | 1.1 s | 1.0 s | 4.3 µs | 331 µs | 424 µs | 364 µs | — | 0.21 GiB |
| ratio | **1.82×** | **2.50×** | **1.44×** | **1.64×** | **1.36×** | **1.08×** | — | |

| 10M | hydrate | settle | get_hit | prefix_scan | get_loop | multi_get | probe_miss p50 | disk |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Pedra | 10.7 s | 0.6 s | 13.0 µs | 261 µs | 1.14 ms | 1.23 ms | — | 2.40 GiB |
| Rocks | 11.1 s | 4.6 s | 13.0 µs | 341 µs | 1.22 ms | 1.26 ms | — | 2.10 GiB |
| ratio | **1.03×** | **7.67×** | 1.00× (tie) | **1.31×** | **1.07×** | **1.02×** | — | |

| 25M | hydrate | settle | get_hit | prefix_scan | get_loop | multi_get | probe_miss p50 | disk |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Pedra | 29.5 s | 0.3 s | 35.2 µs | 248 µs | 3.65 ms | 3.36 ms | — | 5.96 GiB |
| Rocks | 30.2 s | 8.1 s | 40.3 µs | 333 µs | — | 4.29 ms | — | 6.8–8.0 GiB |
| ratio | 1.02× | **27×** | **1.14×** | **1.34×** | — | **1.27×** | — | |

| 100M | hydrate | settle | get_hit | prefix_scan | get_loop | multi_get | probe_miss p50 | disk |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Pedra | 119.4 s | 0.7 s | 63.7 µs | 305 µs | 6.15 ms | 5.84 ms | 211 ns | 24.16 GiB |
| Rocks | 151.3 s | 56.5 s | 68.4 µs | 320 µs | 7.00 ms | 6.70 ms | 571 ns | 20.98 GiB |
| ratio | **1.27×** | **81×** | **1.07×** | **1.05×** | **1.14×** | **1.15×** | **2.71×** | |

Disk is after settle at 1M/10M/100M; the 25M row shows hydrate-time disk
(Rocks's size moves during its settle — at 100M it lands at 20.98 GiB
vs Pedra's 24.16 GiB).

- 10M `get_hit` is a tie (confidence intervals overlap), not a win.
- 25M hydrate sits inside Rocks's own 27.9–34.4 s band and the ±3 s host
  noise — parity, not a win claim.
- 25M `get_loop`: Rocks measured out of its own 3.86–4.03 ms band on
  every attempt (4.17–4.48 ms), so no ratio is published; Pedra's 3.65 ms
  stands as a number, not a claim.
- `probe_miss` at 1M–25M is being measured on the current engine (3
  runs); the cells publish when they exist. That cell's old-engine
  history is a named loss, kept in `docs/benchmarks.md`.
- 100M `prefix_scan`: the middle run tied; the median is what is
  published.

**Fjall** (third peer; not the gate). Same guest, 200 B values,
256 MiB cache. Fjall legs ran on the `scale-parity-bench` harness,
Pedra on `snapshot_backends` — cross-harness ratios, orientation only
(no bold, no gate claim). Ratio = Fjall / Pedra; the Pedra rows repeat
the official numbers above (25M reads: single run; 100M: 3-run medians).

| 25M | hydrate | settle | get_hit | prefix_scan | get_loop | probe_miss p50 | disk |
|---|---:|---:|---:|---:|---:|---:|---:|
| Fjall | 49.8 s | 0.1 s | 40.2 µs | 272.3 µs | 3.60 ms | 1.1 µs | 5.16 GiB |
| Pedra | 29.5 s | 0.3 s | 35.2 µs | 248.5 µs | 3.65 ms | — | 5.96 GiB |
| ratio | 1.69× | 0.33× | 1.14× | 1.10× | 0.99× | — | |

| 100M | hydrate | settle | get_hit | prefix_scan | get_loop | probe_miss p50 | disk |
|---|---:|---:|---:|---:|---:|---:|---:|
| Fjall | 191.4 s | 0.0 s | 82.0 µs | 266.8 µs | 7.82 ms | 1.1 µs | 20.61 GiB |
| Pedra | 119.4 s | 0.7 s | 63.7 µs | 304.8 µs | 6.15 ms | 211 ns | 24.16 GiB |
| ratio | 1.60× | ≈0× | 1.29× | 0.88× | 1.27× | 5.21× | |

Fjall settles during hydrate (≈0 s) and is ahead cross-harness on 100M
`prefix_scan` (0.88×); everywhere else shown, Pedra leads. Fjall legs:
2026-09-04, 3-run medians; per-run values in
[`docs/benchmarks.md`](docs/benchmarks.md).

**Reproducing.** Commands, knobs, and the official leg protocol — smoke,
3 runs, medians, gates — are in
[`docs/benchmarks.md`](docs/benchmarks.md) and
[`crates/snapshot-bench/README.md`](crates/snapshot-bench/README.md).

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
- **Verus twins**: 47 kernel-to-proof pairs over 23 kernels, in 34 proof
  files in the shipped crates, checked against a pinned Verus release with
  `scripts/formal/verus_check.sh --all` — table in
  [Verification](#verification).
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
| `pedradb-examples` | Runnable ladder: hello-world through a small ledger, then backup / Rocks drop-in. |
| `pedradb-ops` | Local backup, WAL shipping, point-in-time restore, format migration. |
| `pedradb-sim` | Seeded fault injection for recovery testing. |
| `pedradb-io-uring` | Linux io_uring `Env` for WAL/SST writes and fsync; POSIX fallback elsewhere. |
| `pedradb-posix` | fdatasync / fallocate / fadvise. With `pedradb-io-uring`, the only `unsafe` in the tree. |
| `rocksdb-compat` | rust-rocksdb 0.22 API on the engine, for migrating existing Rocks code. |
| `rocksdb-parity-bench` | Parity harness: YCSB/deps (`rocks-parity-bench`) and sorted-ingest scale (`scale-parity-bench`). Peers: Pedra, optional RocksDB (`--features real`), optional Fjall (`--features fjall`). |
| `snapshot-bench` | `snapshot_backends` comparative bench (sorted-ingest table): fjall vs RocksDB (`rust-rocksdb` 0.50) vs Pedra, ported from slipstream PR 19 (MIT). Own workspace — see its README. |

MSRV 1.88. Dual-licensed MIT or Apache-2.0. Runnable gallery: [`examples/`](examples/).

## License

Dual-licensed under the MIT license or the Apache License 2.0, at your
option (`LICENSE-MIT` / `LICENSE-APACHE`). This project is not affiliated
with, endorsed by, or derived from the RocksDB source; it is an independent
implementation of compatible concepts and APIs. "RocksDB" is a trademark
of its owners.
