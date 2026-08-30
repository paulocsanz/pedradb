# PedraDB

**Embedded ordered key-value store with multi-key ACID transactions — pure Rust, tiny API, durable by default.**

[![synthetic-field](https://github.com/paulocsanz/pedradb/actions/workflows/synthetic-field.yml/badge.svg)](https://github.com/paulocsanz/pedradb/actions/workflows/synthetic-field.yml)
[![world-nightly](https://github.com/paulocsanz/pedradb/actions/workflows/world-nightly.yml/badge.svg)](https://github.com/paulocsanz/pedradb/actions/workflows/world-nightly.yml)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
![MSRV](https://img.shields.io/badge/rust-1.75%2B-orange.svg)

PedraDB is a storage **kernel**: the smallest engine you can correctly build a
database on. RocksDB gives you a fast LSM but no real transactions — every
system built on it (CockroachDB, TiKV, …) had to reinvent consistency from
scratch. PedraDB puts multi-key ACID in the core, so "update the row *and* its
index" is one atomic, fsynced commit — in-process, no server, no C++.

```text
open → begin → get / put / delete / range → commit
```

## Quick start

```toml
[dependencies]
pedradb-core = { git = "https://github.com/paulocsanz/pedradb" }
```

```rust
use pedradb_core::Db;

fn main() -> pedradb_core::Result<()> {
    let mut db = Db::open("/tmp/pedra-demo")?;

    // Single key, auto-commit
    db.put(b"hello", b"world")?;

    // Multi-key transaction: row + secondary index, atomically
    let mut tx = db.begin();
    tx.put(b"user/42", br#"{"name":"ada"}"#)?;
    tx.put(b"idx/name/ada", b"42")?;
    tx.commit()?; // one WAL record, fsynced before Ok

    assert_eq!(db.get(b"idx/name/ada").as_deref(), Some(b"42".as_ref()));
    db.close()
}
```

Kill the process after `commit` returns `Ok` and reopen the directory — the
committed keys are still there (WAL replay). A crash mid-write never leaves a
partial transaction. Try it:

```sh
cargo run -p pedradb-cli -- demo /tmp/pedra-demo
```

Full walkthrough (durability contract, CAS, change feed, index-layer sketch):
[`docs/usage.md`](docs/usage.md).

## What you get

- **Multi-key ACID transactions** — `begin`/`commit` in the kernel, plus
  optimistic multi-writer transactions (`ConcurrentDb::begin_occ`).
- **Durable by default** — commit fsyncs the WAL before returning `Ok`; a
  failed sync *fences* the handle instead of silently continuing.
- **Ordered keys** — streaming `scan`, paginated `range_limited`, key-only
  projections.
- **Snapshots and MVCC** — `get_at`/`scan_at` at a commit sequence, pinnable
  snapshots, and a durable **change feed** (`changes`/`changes_after`) for
  watch/CDC layers.
- **First-class CAS** — `put_if_absent`, `put_if_eq`, `compare_and_swap`; the
  hooks leases and leader locks need, without DIY read-modify-write.
- **A real LSM underneath** — WAL, MemTable, SSTs with Bloom filters + lz4,
  leveled compaction, MANIFEST, checksum verification; opt-in WiscKey-style
  value log for large values with crash-safe GC.
- **Bounded history by default** — a 24 h MVCC window with a CRC'd cold
  archive (optionally mirrored to object storage); old snapshots fail with a
  typed `SnapshotTooOld`, never a silent destroy.
- **Ops built in** — base + incremental backup, WAL shipping, point-in-time
  restore, checkpoints, format migration, `pedra` CLI.
- **Pure Rust** — `#![forbid(unsafe_code)]` in the kernel. RocksDB is used
  only as an external *test oracle*; no C++ is linked.
- **io_uring on Linux** — production opens use `io_uring` for write + fsync,
  with POSIX fallback elsewhere.
- **RocksDB drop-in** — the `rocksdb-compat` crate mirrors the `rust-rocksdb`
  API with [documented divergences](docs/rocksdb-compat.md).

Encryption at rest is a non-goal in the engine: use volume encryption
(LUKS, FileVault) or app-layer AEAD.

## How it's tested

Correctness is the product, so most of this repo is verification machinery:

- **Deterministic simulation** (`pedradb-world`): whole clusters run inside a
  seeded simulator — same seed, same `trace_hash`, byte-for-byte. Nightly
  swarm campaigns replay thousands of seeds (16 384 @ 3 nodes, 4 096 @ 7 and
  9 nodes) with fault injection and cross-node invariant checks
  (split-brain, resurrection, silent-wrong = 0).
- **Fault injection at the `Env` seam** (`pedradb-sim`): lying fsync, short
  writes, torn WAL tails, injected I/O errors — the same seams the simulator
  and the real engine share.
- **Oracle testing** (`pedradb-oracle`): workloads are diffed against a model
  and, optionally, against real RocksDB.
- **Concurrency schedule exploration**: PCT-style randomized schedules over
  the real `ConcurrentDb`, validated by planted bugs it must find.
- **Sanitizer boxes**: Miri, TSan, and ASan run as sibling CI jobs.
- **Formally verified kernels**: the group-commit and OCC commit paths are
  proven in Verus and Lean; `OpenOptions::verified()` runs the critical
  sections on the extracted, machine-checked kernels
  ([RFC-0058](docs/rfc/0058-verified-mode-kernel-derived-fallback.md)).

What this does **not** claim: field parity with RocksDB, Pebble, or FDB.
Those engines have years of production exposure PedraDB does not — the honest
comparison lives in
[`docs/robustness-vs-rocks-pebble-fdb.md`](docs/robustness-vs-rocks-pebble-fdb.md).

## Beyond one machine: MontanhaDb

The kernel is local-only on purpose. **[MontanhaDb](docs/montanhadb.md)**
(*Montan-HA-DB*) is the multi-node product family built by *embedding* it:

```text
  app / SQL / streams / HTTP / DCS      ← layers (each is a crate)
        │
  MontanhaDb: multi-Raft ranges, TCP    ← pedradb-store, montanha-tcp
        │
  PedraDB kernel                        ← pedradb-core (this library)
```

| Area | Crates |
|------|--------|
| Kernel | `pedradb-core`, `pedradb-io-uring`, `pedradb-posix` |
| Ops | `pedradb-ops`, `pedradb-cli` (`pedra`) |
| RocksDB compat | `rocksdb-compat`, `rocksdb-parity-bench` |
| Distribution | `pedradb-raft`, `pedradb-store`, `montanha-tcp`, `pedradb-dcs`, `pedradb-replicate`, `pedradb-apply` |
| Layers | `pedradb-sql`, `pedradb-http`, `pedradb-stream`, `pedradb-fold`, `pedradb-lease`, `pedradb-index`, `pedradb-journal` |
| Verification | `pedradb-sim`, `pedradb-oracle`, `pedradb-world`, `pedradb-dst` |

North star: a Postgres-class platform that replaces the *need* for
Scylla + ClickHouse + NATS in one product — see
[`docs/node-primitive-and-unified-platform.md`](docs/node-primitive-and-unified-platform.md).

## When to use it (and when not)

Reach for PedraDB when **all** of these are true: you want an embedded
library (not a server), durable ordered KV, multi-key atomic commits, a small
API, and Rust without C++ in the hot path.

| If you need… | Use instead |
|---|---|
| A cache / ephemeral store | Redis |
| A stable B-tree embed, today | redb / heed |
| Distributed SQL out of the box | TiDB / CockroachDB |
| S3-bottomless embed | SlateDB / Tonbo |

Non-goals in the kernel (layers do these): SQL, secondary indexes, network
server, multi-node, hundreds of tunables. Why:
[`docs/positioning.md`](docs/positioning.md).

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo bench -p pedradb-core --bench baseline
```

Multi-host clusters, swarm campaigns, docker smoke, backup/PITR recipes:
[`docs/status.md`](docs/status.md#reproduce--advanced-commands).

## Documentation

- [`docs/usage.md`](docs/usage.md) — **start here**: open, transactions, durability contract, layer sketches
- [`docs/positioning.md`](docs/positioning.md) — why this exists; power/surface ratio
- [`docs/architecture.md`](docs/architecture.md) — architecture and roadmap
- [`docs/status.md`](docs/status.md) — detailed per-crate status + RFC ledger
- [`docs/rfc/`](docs/rfc/) — the full RFC program, from [0001](docs/rfc/0001-pedradb-high-level-spec.md)
- [`docs/montanhadb.md`](docs/montanhadb.md) — the multi-node product family
- [`docs/robustness-vs-rocks-pebble-fdb.md`](docs/robustness-vs-rocks-pebble-fdb.md) — honest robustness comparison
- [`docs/open-items.md`](docs/open-items.md) — living list of open items
- [`docs/references/`](docs/references/) — primary sources (papers, docs)

## License

Apache-2.0
