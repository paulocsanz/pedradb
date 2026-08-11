# PedraDB

**Why use it:** multi-key ACID + ordered KV, in-process, tiny API — so you can
update data (and indexes) correctly without a cluster or C++ RocksDB.

```text
open → begin → get / put / delete / range → commit
```

Small surface. Fast path. Build on top.  
No server, no multi-node, no SQL. Simulation and research LSM come **after**
that use is real — they don’t justify the crate by themselves.
See [`docs/positioning.md`](docs/positioning.md).

```
  App or future multi-node DB              PedraDB (this repo)
        │                                        │
        └── embeds ────────────────────────────► │  local ACID ordered KV
                                                 │  (RocksDB’s role + TX)
```

**Spec (normative high-level):** [`docs/rfc/0001-pedradb-high-level-spec.md`](docs/rfc/0001-pedradb-high-level-spec.md)  
**Open decisions deep dive:** [`docs/rfc/0001-open-decisions-deep-dive.md`](docs/rfc/0001-open-decisions-deep-dive.md)  
**Session synthesis + strategic doubt:** [`docs/session-synthesis-architecture-and-doubt.md`](docs/session-synthesis-architecture-and-doubt.md)  
**Grail plan (DBs on PedraDB, incl. etcd-class):** [`docs/grail-plan-build-databases-on-pedradb.md`](docs/grail-plan-build-databases-on-pedradb.md)  
**Doctrine (primitive + API layers only):** [`docs/doctrine-primitives-and-api-layers.md`](docs/doctrine-primitives-and-api-layers.md)  
**Plug map (replace etcd/SQLite/PG/TiKV/…):** [`docs/plug-map-replace-incumbents.md`](docs/plug-map-replace-incumbents.md)  
Focus: [`docs/positioning.md`](docs/positioning.md) · Architecture:
[`docs/architecture-refined.md`](docs/architecture-refined.md).

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

- [`docs/positioning.md`](docs/positioning.md) — **focus**: small surface, speed, build-on power
- [`docs/architecture.md`](docs/architecture.md) — architecture and roadmap
- [`docs/architecture-refined.md`](docs/architecture-refined.md) — local-only substrate role
- [`docs/fdb-limitations-analysis.md`](docs/fdb-limitations-analysis.md) — why PedraDB solves what FDB can't
- [`docs/engine-landscape-and-ideal-path.md`](docs/engine-landscape-and-ideal-path.md) — engine comparison
- [`docs/distributed-systems-analysis.md`](docs/distributed-systems-analysis.md) — ScyllaDB, Ceph, TiKV, FDB analysis
- [`docs/distribution-design.md`](docs/distribution-design.md) — how embedded PedraDB becomes distributed
- [`docs/distribution-deep-research.md`](docs/distribution-deep-research.md) — Percolator, Parallel Commits, PD, TSO, Raft
- [`docs/scylladb-architecture.md`](docs/scylladb-architecture.md) — how ScyllaDB operates (AP multi-master NoSQL)
- [`docs/tidb-architecture.md`](docs/tidb-architecture.md) — TiDB as SQL layer on TiKV/PD/TiFlash
- [`docs/foundationdb-layers-and-products.md`](docs/foundationdb-layers-and-products.md) — DBs and products built on FoundationDB
- [`docs/etcd-comparison.md`](docs/etcd-comparison.md) — etcd vs all systems in this research
- [`docs/competitive-landscape-rust.md`](docs/competitive-landscape-rust.md) — Rust peers (fjall, SurrealKV, redb, …)
- [`docs/compare-fjall.md`](docs/compare-fjall.md) — PedraDB vs fjall
- [`docs/rocksdb-critiques-and-improvements.md`](docs/rocksdb-critiques-and-improvements.md) — detailed critiques
- [`docs/open-items.md`](docs/open-items.md) — living list of open items and status
- [`docs/references/`](docs/references/) — all primary sources (papers, docs)

License: Apache-2.0.
