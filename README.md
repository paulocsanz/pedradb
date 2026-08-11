# PedraDB

A **transactional key-value storage engine** in Rust, designed as a foundation
for building databases.

PedraDB is not another KV store. It is a **pillar** upon which other databases
(SQL, document, graph, time-series) are constructed — inspired by FoundationDB's
layer concept. The core exposes ordered key-value with ACID transactions and
nothing else. Database builders create layers on top, using transactions to
guarantee consistency.

```
┌───────────────────────────────────────────────┐
│   SQL DB · Document DB · Graph DB · ...       │  ← user-built layers
├───────────────────────────────────────────────┤
│   PedraDB: ordered KV + ACID transactions     │  ← the pillar
├───────────────────────────────────────────────┤
│   LSM engine (WiscKey + Monkey + Dostoevsky)  │  ← implementation detail
└───────────────────────────────────────────────┘
```

## Why transactions at the core

RocksDB has no ACID transactions. Every database built on it (CockroachDB, TiKV)
had to reinvent distributed consistency from scratch — years of engineering each.
PedraDB puts transactions in the core so database builders never have to solve
consistency themselves.

## Status

| Slice | Status |
|-------|--------|
| WAL (crash-safe log) | ✅ |
| InternalKey + MemTable | ⏳ next |
| Transaction manager | 🔲 |
| Transactional API | 🔲 |
| SST + flush (WiscKey + Monkey) | 🔲 |
| Get + range scan | 🔲 |
| Compaction (Lazy Leveling) | 🔲 |
| Version GC | 🔲 |
| Deterministic simulation | 🔲 |
| Cross-validation harness | 🔲 |

See [`docs/architecture.md`](docs/architecture.md) for the full design and
[`docs/open-items.md`](docs/open-items.md) for the complete status tracker.

## Build & test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo run -p pedradb-cli -- wal /tmp/demo.log
```

## Documentation

- [`docs/architecture.md`](docs/architecture.md) — full architecture and roadmap
- [`docs/fdb-limitations-analysis.md`](docs/fdb-limitations-analysis.md) — why PedraDB solves what FDB can't
- [`docs/engine-landscape-and-ideal-path.md`](docs/engine-landscape-and-ideal-path.md) — engine comparison
- [`docs/distributed-systems-analysis.md`](docs/distributed-systems-analysis.md) — ScyllaDB, Ceph, TiKV, FDB analysis
- [`docs/distribution-design.md`](docs/distribution-design.md) — how embedded PedraDB becomes distributed
- [`docs/rocksdb-critiques-and-improvements.md`](docs/rocksdb-critiques-and-improvements.md) — detailed critiques
- [`docs/open-items.md`](docs/open-items.md) — living list of open items and status
- [`docs/references/`](docs/references/) — all primary sources (papers, docs)

License: Apache-2.0.
