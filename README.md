<h1 align="center">PedraDB</h1>

<p align="center">
  <b>An embeddable key-value store. Multi-key transactions are the API.</b><br>
  Pure Rust. Durable by default. Fail-closed.
</p>

<p align="center">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="license: MIT OR Apache-2.0"></a>
  <a href="https://github.com/paulocsanz/pedradb/actions/workflows/ci.yml"><img src="https://github.com/paulocsanz/pedradb/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/rust-1.88%2B-orange.svg" alt="MSRV 1.88">
  <img src="https://img.shields.io/badge/status-alpha-yellow.svg" alt="status: alpha">
</p>

You link it into your process. Keys and values are bytes, stored sorted on
local disk. A commit writes the row and its index as one WAL record and
fsyncs before `Ok`. The engine is `#![forbid(unsafe_code)]`. Status is
alpha: the format and the API can still break.

The only speed claim is against **RocksDB default**
(`WriteOptions.sync=false`), on Linux. A ratio above 1 means Pedra is
faster. macOS numbers are not the claim. A win against `sync=true` is not
a win. Per-run values, the loss list, and how to reproduce a cell are in
[`docs/benchmarks.md`](docs/benchmarks.md).

## Quickstart

```toml
[dependencies]
pedradb-core = { git = "https://github.com/paulocsanz/pedradb" }
```

```rust
use pedradb_core::ConcurrentDb;

let mut db = ConcurrentDb::open("/tmp/pedra")?;
let mut tx = db.begin_occ();
tx.put(b"user/42", br#"{"name":"ada"}"#)?;
tx.put(b"idx/name/ada", b"42")?;
tx.commit()?;
assert_eq!(db.get(b"idx/name/ada").as_deref(), Some(b"42".as_ref()));
```

```sh
cargo run -p pedradb-examples --example hello
```

`rocksdb-compat` is the rust-rocksdb 0.22 surface, for trying an existing
caller. It is a migration path, not the product, and it defaults to
Rocks's async WAL so a comparison is the same durability class.

## Verification

Decision kernels are proved on the file `rustc` links (Charon + Aeneas →
Lean). The catalog has **151 pairs**, each one the shipped function, not a
model written beside it. CI runs `scripts/formal/pedra_formal.py --lint`:
drift between a kernel and its extract stamp fails the build, and the
current tree is **0 FAIL** (the ratchet still rejects a non-drift FAIL
count above 107). The engine crate has **1,094 passing tests** (4 ignored).

Not proved: the OS, the disk, rustc, Aeneas, Lean, or Z3. Store and raft
are not in this repository, so their proofs are not either.

The catalog map, what each check refuses, and how to run it:
[`docs/verification.md`](docs/verification.md).

## Benchmarks

Linux, 4 vCPU on a Threadripper PRO 3975WX, RocksDB default
`sync=false`. Bold is a win. Plain is a tie or parity. A dash is not
measured on the current engine, or the peer was outside its own band.

**Same durability as production Rocks** (Pedra WAL `write()`, no fsync
per operation). September 2026.

| Workload | Median | Min | Rounds |
|---|---:|---:|---|
| Overwrite, 25M keys, 4 clients | **1.32×** | 1.06× | 2 of 3 |
| Batched writes, 4 clients | **1.23×** | 1.19× | 3/3 |
| Read-heavy mix, 4 clients (95% reads) | **1.13×** | 1.00× | 3/3 |
| Lookup of a missing key, 100M keys | **17.6×** | 14.4× | 2 of 3 |
| Read-modify-write, 4 clients | 0.70× | 0.67× | open |

Read-modify-write is the open loss. A 100M prefix scan on a 4 GiB guest
with a bounded cache is a separate open cell at **0.70×**. An earlier
same-class battery (25 Aug 2026, 17 shapes, every minimum above 1.0×,
tightest 1.014× on a replicated-log append) is in
[`docs/benchmarks.md`](docs/benchmarks.md).

The default build still fsyncs before `Ok`, which is a stronger barrier
than this peer. Reads on that column stay ahead (about 1.1–2.0×). A
single client paying one fsync per write is slower than Rocks with no
barrier; that is the price of the default, not a win. Under concurrency
the same batched-write shape on that stronger column was **2.79×**.

**Sorted ingest.** Clustered keys, 200-byte values, 256 MiB cache, one
process per engine. 1M and 10M are one official run (2 Sep). 25M load is
three runs (3 Sep); its reads are one run. 100M is three runs (5 Sep).

| | 1M | 10M | 25M | 100M |
|---|---:|---:|---:|---:|
| Load | **1.82×** | **1.03×** | 1.02× | **1.27×** |
| Settle | **2.50×** | **7.67×** | **27×** | **81×** |
| Point read | **1.44×** | tie | **1.14×** | **1.07×** |
| Prefix scan | **1.64×** | **1.31×** | **1.34×** | **1.05×** |
| 100-key read | **1.36×** | **1.07×** | — | **1.14×** |
| Multi-get | **1.08×** | **1.02×** | **1.27×** | **1.15×** |
| Absent-key probe (p50) | — | — | — | **2.71×** |

25M load is parity inside host noise. 10M point read is a tie (the
intervals overlap). 25M 100-key read has no ratio: Rocks sat outside its
own band on every attempt. Absent-key probes at 1M and 10M were measured
on an older engine — 10M was **0.39×**, a loss — and have not been
re-run; 25M was not measured. At 100M on the current engine the probe is
211 ns versus 571 ns. Absolute times and the loss list are in
[`docs/benchmarks.md`](docs/benchmarks.md).

Fjall, same binary, not the Rocks gate: random read/write at 1M keys
**1.03×** (min 1.02×), a 1,024-key scan **1.09×** (one round 0.996×).

## Crates

| Crate | What it is |
|---|---|
| `pedradb-core` | The engine. `#![forbid(unsafe_code)]`. |
| `pedradb-spec` | Durability, no-resurrection, and atomicity predicates linked by rustc. |
| `pedradb-ops` | Backup, WAL shipping, point-in-time restore. |
| `pedradb-sim` | Seeded I/O faults on the real recovery path. |
| `pedradb-posix`, `pedradb-io-uring` | The only `unsafe` in the tree: fdatasync / fallocate / fadvise, and the Linux ring. |
| `rocksdb-compat` | rust-rocksdb 0.22 API. |
| `pedradb-examples` | hello-world through a small ledger. |

MSRV 1.88.

## License

MIT or Apache-2.0, at your option. Not affiliated with RocksDB.
