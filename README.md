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

Keys and values are bytes, stored sorted on
local disk. A commit writes the row and its index as one WAL record and
fsyncs before `Ok`. The engine is `#![forbid(unsafe_code)]`. Status is
alpha: the format and the API can still break.

The speed claim is Pedra against **RocksDB default**
(`WriteOptions.sync=false`) and against **fjall**, on Linux. A ratio
above 1 means Pedra is faster. macOS numbers are not the claim. Per-run
values, the loss list, and how to reproduce a cell are in
[`docs/benchmarks.md`](docs/benchmarks.md).

## Quickstart

```toml
[dependencies]
pedradb-core = { git = "https://github.com/paulocsanz/pedradb" }
```

```rust
use pedradb_core::ConcurrentDb;

let db = ConcurrentDb::open("/tmp/pedra")?;
let mut tx = db.begin_occ();
tx.put(b"user/42", br#"{"name":"ada"}"#)?;
tx.put(b"idx/name/ada", b"42")?;
tx.commit()?;
assert_eq!(db.get(b"idx/name/ada").as_deref(), Some(b"42".as_ref()));
```

```sh
cargo run -p pedradb-examples --example hello
```

> **Durability note**: PedraDB defaults to `fdatasync` before returning `Ok` on commits.
> A single client executing 1-key commits pays the physical drive barrier (~1–3 ms on NVMe).
> For high throughput, use multi-key transactions (`begin_occ`), concurrent worker threads
> (where group commit amortizes the barrier across writers), or `rocksdb-compat` for async WAL.

`rocksdb-compat` is the rust-rocksdb 0.22 surface, for trying an existing
caller. It is a migration path, not the product, and it defaults to
Rocks's async WAL so a comparison is the same durability class.

## Verification

Decision kernels are proved on the file `rustc` links (Charon + Aeneas →
Lean). The catalog has **172 pairs**, each one the shipped function, not a
model written beside it. CI runs `scripts/formal/pedra_formal.py --lint`
in its own job (seconds — never starved by a long test build): drift
between a kernel and its extract stamp fails the build, and any drift or
unclassified public surface fails CI (debt ceiling 0 — a new unclassified
function is a red build, not a warning). The engine crate has **1,325
passing tests** (4 ignored); the CI also runs the `rocksdb-compat`
differential oracle and adversarial suites.

Decision kernels enforce the **Three Teeth Principle** (RFC-0151 / RFC-0329):
synthetic mutants planted in the decision kernels are killed with a 100% kill
rate by automated test oracles using zero-recompile mutation switching.

This repository ships the embedded engine and its operational surface
(`pedradb-core`, `-posix`, `-io-uring`, `-sim`, `-spec`, `-ops`,
`rocksdb-compat`, and the bench harnesses). The distributed building
blocks under development (store, raft, sql, http, replication) live in
the development tree and are not claimed here.

Not proved: the OS, the disk, rustc, Aeneas, Lean, or Z3.

The catalog map, what each check refuses, and how to run it:
[`docs/verification.md`](docs/verification.md).

## Observability & Health

PedraDB provides zero-lock-contention health diagnostics and internal metrics directly from `pedradb-core`:
- **Tri-State Health Model**: `Healthy`, `Degraded`, and `ActionRequired` evaluations via `db.health()`.
- **Diagnostic Issues & Remediations**: Automatic detection of L0/memtable write stalls, snapshot pin leaks (analogous to Postgres `datfrozenxid` wraparound risk), cache thrashing, and corruption events, paired with typed remediation recommendations.
- **Pure Expositions**: OpenMetrics/Prometheus (`format_prometheus_metrics`) and structured JSON (`format_json_status`) formatting without external telemetry dependencies.
- Detailed guide: [`docs/metrics.md`](docs/metrics.md).

## Concurrency & Durability Guarantees

- **Strict Read-Your-Writes Linearizability**: Every acknowledged transaction (`tx.commit()` -> `Ok`) is immediately visible to subsequent `get` calls on the same thread and concurrent observers. `ConcurrentDb::get` falls back to verified read-lock acquisition if optimistic SuperVersion publication is mid-transition, eliminating transient stale reads (`crates/pedradb-core/tests/rfc0330_strict_linearizability_read_your_writes.rs`).
- **Gapless Anti-Hole Crash Consistency**: Positional WAL allocations for asynchronous writers are bound to RAII anti-hole seals (Contract F182). Aborted or cancelled write jobs automatically seal allocated spans with valid NOP frames, preventing mid-log tearing on crash recovery (`crates/pedradb-core/tests/rfc0330_async_wal_anti_hole_contract.rs`).
- **Fail-Closed Verification**: Checksums (CRC32C) and monotonic sequence numbers guard all WAL records, manifest updates, and SST blocks. Any uncorrectable physical corruption aborts recovery safely rather than serving damaged data.

## Status (alpha)

- Status is alpha: the on-disk format and API surface can still evolve before 1.0.
- Open benchmark cells are named, not hidden: [Named losses](#named-losses).

## Benchmarks

Linux, single guest (4 vCPU on a Threadripper PRO 3975WX host, NVMe).
Ratio = Pedra ÷ peer; above 1 means Pedra is faster. **Bold** is a win;
plain is a tie or parity; `—` is unpublished — not measured, or the peer
sat outside its own historical band and the cell was refused. Protocol,
per-run values behind every median, and the full loss registry:
[`docs/benchmarks.md`](docs/benchmarks.md).

### Scale ladder — sorted ingest, vs RocksDB default

Clustered keys, 200 B values, 256 MiB cache, one backend per process.
1M and 10M are one official run (2 Sep 2026); 25M load is three runs and
its reads one run (3 Sep); 100M is three runs (5 Sep).

| op | 1M | 10M | 25M | 100M |
|---|---:|---:|---:|---:|
| Load | **1.82×** | **1.03×** | 1.02× | **1.27×** |
| Settle | **2.50×** | **7.67×** | **27×** | **81×** |
| Point read | **1.44×** | tie | **1.14×** | **1.07×** |
| Prefix scan | **1.64×** | **1.31×** | **1.34×** | **1.05×** |
| 100-key read | **1.36×** | **1.07×** | — ¹ | **1.14×** |
| Multi-get | **1.08×** | **1.02×** | **1.27×** | **1.15×** |
| Absent-key probe (p50) | — ² | — ² | — ² | **2.71×** |

25M load is parity inside ±3 s of host noise. 10M point read: the
intervals overlap. At 100M the absent-key probe is 211 ns vs 571 ns
(p99: 231–311 ns vs 842 ns–2.4 µs).

¹ Refused: Rocks measured outside its own band on every attempt.
² The 2 Sep values predate the per-column-family envelope fix — 10M was
a **0.39×** loss on that engine — and 25M was never measured.
Re-measuring on the current engine.

### Client mixes — 4 concurrent clients

Same durability class: Pedra async WAL against Rocks default
(`WriteOptions.sync=false`), September 2026. The fjall rows are a second
peer (same binary, both sides buffering the measured window, 21 Sep).

| shape | peer | median | min | rounds |
|---|---|---:|---:|---|
| Overwrite, 25M keys | Rocks | **1.32×** | 1.06× | 2 of 3 ³ |
| Batched writes | Rocks | **1.23×** | 1.19× | 3/3 |
| Read-heavy mix, 95% reads | Rocks | **1.13×** | 1.00× | 3/3 |
| Missing-key lookup, 100M keys | Rocks | **17.6×** | 14.4× | 2 of 3 ³ |
| Random read/write, 1M keys | fjall | **1.03×** | 1.02× | 3/3 |
| Scan, 1,024 keys | fjall | **1.09×** | 0.996× | 3/3 |

³ One round discarded: the Rocks canary sat under that wave's floor.

### Named losses

- **Read-modify-write, 4 clients: 0.70×** (min 0.67×) — open.
- **Prefix scan, 100M keys, memory-bounded guest** (4 GiB RAM, 256 MiB
  cache): **0.70×** vs RocksDB's block cache — open. A different cell
  from the cache-warm 1.05× ladder row above.
- **Single-client write-per-op: below 1× by construction.** The default
  build `fdatasync`s before `Ok` — one physical barrier per operation
  against the peer's zero. Under 4-client concurrency the same
  batched-write shape is **2.79×** (group commit amortizes the barrier),
  and reads on the default build stay 1.1–2.0× ahead.

An earlier same-class battery (25 Aug 2026, 17 shapes, every minimum
above 1.0×, tightest 1.014× on a replicated-log append) is in
[`docs/benchmarks.md`](docs/benchmarks.md).

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
