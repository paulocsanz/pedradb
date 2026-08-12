# PedraDB

**Why use it (kernel):** multi-key ACID + ordered KV, in-process, tiny API — so you can
update data (and indexes) correctly without C++ RocksDB.

**North star (platform):** a **Postgres-class system** that replaces the **need** for
**Scylla + ClickHouse + NATS** in one product (multi-primary, OLTP + OLAP RO + streams).  
See [`docs/node-primitive-and-unified-platform.md`](docs/node-primitive-and-unified-platform.md) §1.

**Product names:**

| Name | Meaning |
|------|---------|
| **PedraDB** | This **kernel** (embed library) — local storage primitive per node |
| **[MontanhaDb](docs/montanhadb.md)** (**Montan-HA-DB**) | **HA / multi-node** product family on PedraDB (multi-Raft, N leaders, platform face) |

Layers (Raft, DCS, SQL subset, streams, HTTP) ship as crates that **embed** the kernel and roll up under **MontanhaDb**.

```text
open → begin → get / put / delete / range → commit
```

Small kernel surface. Fast path. Build on top.  
See [`docs/positioning.md`](docs/positioning.md).

```
  App / pedradb-raft / pedradb-http / sql / stream
        │
        └── embeds ────────────────────────────► pedradb-core
                                                 local ACID ordered KV
```

**RFCs:** [`docs/rfc/`](docs/rfc/) — start with
[0001](docs/rfc/0001-pedradb-high-level-spec.md), engine
[0009](docs/rfc/0009-rocksdb-class-engine.md), maturity wave 2
[0014](docs/rfc/0014-rocks-pebble-redwood-maturity.md), audit
[0015](docs/rfc/0015-audit-pedradb-correctness-fixes.md), production robustness
[0016](docs/rfc/0016-pedradb-production-robustness.md), Montanha FDB-class
[0017](docs/rfc/0017-montanha-fdb-class-substrate.md), products
[0010](docs/rfc/0010-dbs-on-top.md), faults
[0011](docs/rfc/0011-env-fault-injection.md), multi-node/wire
[0012](docs/rfc/0012-next-significant-steps.md).  
**Usage:** [`docs/usage.md`](docs/usage.md) · **Apply/Raft:** [`docs/apply-and-raft.md`](docs/apply-and-raft.md)  
**MontanhaDb (Montan-HA-DB):** [`docs/montanhadb.md`](docs/montanhadb.md) · deep research [`docs/montanhadb-deep-research.md`](docs/montanhadb-deep-research.md)  
**Live leadership / Patroni-shaped HA:** [`docs/live-leadership-and-patroni-shaped-ha.md`](docs/live-leadership-and-patroni-shaped-ha.md)  
**DCS market / anti-etcd footguns:** [`docs/dcs-market-landscape.md`](docs/dcs-market-landscape.md) · [`docs/multi-node-without-etcd-footguns.md`](docs/multi-node-without-etcd-footguns.md)  
**Perf ceiling / anti-corner / sled-shaped layer:** [`docs/performance-ceiling-option-preservation-and-sled-layer.md`](docs/performance-ceiling-option-preservation-and-sled-layer.md)  
**Node primitive → unified platform (SQLite…CH…multi-leader grail):** [`docs/node-primitive-and-unified-platform.md`](docs/node-primitive-and-unified-platform.md)  
**HTAP research (triangle, LASER/TiFlash/PolarDB, hooks):** [`docs/htap-storage-primitives-and-research.md`](docs/htap-storage-primitives-and-research.md)  
**Replace Scylla *need* (control plane, not CQL):** [`docs/scylla-need-replacement.md`](docs/scylla-need-replacement.md)

## Why transactions at the core

RocksDB has no ACID transactions. Every database built on it (CockroachDB, TiKV)
had to reinvent distributed consistency from scratch — years of engineering each.
PedraDB puts transactions in the core so database builders never have to solve
consistency themselves.

## Usage

```sh
cargo run -p pedradb-cli -- demo /tmp/pedra-demo
```

Library quickstart and secondary-index sketch: [`docs/usage.md`](docs/usage.md).

```rust
use pedradb_core::Db;

let mut db = Db::open("/tmp/pedra")?;
let mut tx = db.begin();
tx.put(b"row", b"data")?;
tx.put(b"idx", b"ptr")?;
tx.commit()?;
db.flush()?; // MemTable → SST (optional; shrinks WAL/mem)
```

Default commit **fsyncs the WAL** before `Ok` (process-crash safe after successful commit).

## Status (shipped)

| Layer | Crates | Status |
|-------|--------|--------|
| Kernel LSM + TX | `pedradb-core` | ✅ WAL, MemTable, **SST v4 + Bloom + lz4**, leveled compact, MANIFEST v2, LOCK, checkpoint/stats/verify, scan, ConcurrentDb, range delete |
| Fault injection | `pedradb-sim` | ✅ FailingEnv, RecordingEnv, lying sync, short-write, seed, Arc |
| Oracle | `pedradb-oracle` | ✅ model (+ optional RocksDB) |
| Ordered apply | `pedradb-apply` | ✅ LogApplier, FakeLog, KvService, InProcessCluster |
| Raft (TCP + persist) | `pedradb-raft`, `pedra-raft-node` | ✅ elect, put, multi-process, failover |
| **Montanha-Store** multi-Raft | `pedradb-store` | ✅ ranges + put/get + **DCS on store** + Queued RPC |
| WAL ship replica | `pedradb-replicate` | ✅ |
| DCS SM | `pedradb-dcs` | ✅ CAS, lease, watch, leader lock |
| HTTP wire | `pedradb-http` | ✅ KV + DCS |
| SQL subset | `pedradb-sql` | ✅ CREATE/INSERT/SELECT/DELETE (not PG wire) |
| Durable stream | `pedradb-stream` | ✅ publish + consumer cursor |
| DST seed sweep | `pedradb-dst` | ✅ |
| **io_uring Env** | `pedradb-io-uring` | ✅ Linux write+fsync; POSIX fallback elsewhere |
| **Ops (backup/PITR/migrate)** | `pedradb-ops`, `pedra` CLI | ✅ base backup, ship_wal, restore, PITR, inspect, migrate |
| Toward Rocks/Pebble/Redwood | [RFC-0014](docs/rfc/0014-rocks-pebble-redwood-maturity.md) | done P0–P2 (OCC, vlog spill, incremental backup) |
| Audit durability / Env fixes | [RFC-0015](docs/rfc/0015-audit-pedradb-correctness-fixes.md) | done (P0–P2) |
| Pedra production robustness | [RFC-0016](docs/rfc/0016-pedradb-production-robustness.md) | P0 done — `compact_vlog`, stats, soak; P1 group-commit done; P1.4/P2.1 open |
| Montanha FDB-class substrate | [RFC-0017](docs/rfc/0017-montanha-fdb-class-substrate.md) | draft — multi-process majority, cluster DST, placement |
| L1 primitive for platform + Scylla-need | [RFC-0019](docs/rfc/0019-local-primitive-for-platform-and-scylla-need.md) | done (P0–P2.2) — CAS, seq pin, change feed, multi_get, soak, compact_for_reads |

RFCs **0001–0012**, **0014**, **0015** delivered. **0016/0017** are the next robustness + cluster waves (not field parity claims).

## Build & test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo bench -p pedradb-core --bench baseline
cargo run -p pedradb-cli -- demo /tmp/pedra-demo
# Local backup / PITR / migrate
cargo run -p pedradb-cli -- backup /tmp/pedra-demo /tmp/pedra-bak
cargo run -p pedradb-cli -- ship-wal /tmp/pedra-demo /tmp/pedra-bak   # after more durable writes (still in WAL)
cargo run -p pedradb-cli -- pitr /tmp/pedra-bak 1 <seq> /tmp/pedra-restored
cargo run -p pedradb-cli -- migrate /tmp/pedra-demo
# Linux: open with io_uring Env
# use pedradb_io_uring::open("/data/pedra")
# 3-node Raft (example ports)
cargo build -p pedradb-raft --bin pedra-raft-node
# cargo run -p pedradb-raft --bin pedra-raft-node -- --id 1 --data /tmp/r1 --bind 127.0.0.1:17001 --peer 1=... --peer 2=... --peer 3=...
```

## Documentation

- [`docs/usage.md`](docs/usage.md) — **start here**: open, TX, durability, index sketch
- [`docs/positioning.md`](docs/positioning.md) — **focus**: small surface, speed, build-on power
- [`docs/architecture.md`](docs/architecture.md) — architecture and roadmap
- [`docs/architecture-refined.md`](docs/architecture-refined.md) — local-only substrate role
- [`docs/fdb-limitations-analysis.md`](docs/fdb-limitations-analysis.md) — why PedraDB solves what FDB can't
- [`docs/engine-landscape-and-ideal-path.md`](docs/engine-landscape-and-ideal-path.md) — engine comparison
- [`docs/distributed-systems-analysis.md`](docs/distributed-systems-analysis.md) — ScyllaDB, Ceph, TiKV, FDB analysis
- [`docs/distribution-design.md`](docs/distribution-design.md) — how embedded PedraDB becomes distributed
- [`docs/distribution-deep-research.md`](docs/distribution-deep-research.md) — Percolator, Parallel Commits, PD, TSO, Raft
- [`docs/scylladb-architecture.md`](docs/scylladb-architecture.md) — how ScyllaDB operates (AP multi-master NoSQL)
- [`docs/scylla-need-replacement.md`](docs/scylla-need-replacement.md) — replace the *need* for Scylla (routes/orchestrator), not CQL drop-in
- [`docs/tidb-architecture.md`](docs/tidb-architecture.md) — TiDB as SQL layer on TiKV/PD/TiFlash
- [`docs/tidb-vs-postgres-mysql.md`](docs/tidb-vs-postgres-mysql.md) — TiDB vs Postgres vs MySQL (when each wins)
- [`docs/sql-lessons-for-the-grail.md`](docs/sql-lessons-for-the-grail.md) — lessons from Postgres/MySQL + Aurora/Neon/Vitess/Citus/Spanner for the grail ladder
- [`docs/object-storage-as-substrate-possibility.md`](docs/object-storage-as-substrate-possibility.md) — SlateDB/WarpStream/turbopuffer/Tigris researched as a possibility for Rung 1.5 (not the kernel), with nuances
- [`docs/conversation-learnings-and-short-term-alignment.md`](docs/conversation-learnings-and-short-term-alignment.md) — conversation learnings + **P0 conflict check**
- [`docs/nats-need-replacement.md`](docs/nats-need-replacement.md) — replace JetStream *need* (durable log), not Core NATS; Jepsen 2.12.1
- [`docs/foundationdb-layers-and-products.md`](docs/foundationdb-layers-and-products.md) — DBs and products built on FoundationDB
- [`docs/etcd-comparison.md`](docs/etcd-comparison.md) — etcd vs all systems in this research
- [`docs/competitive-landscape-rust.md`](docs/competitive-landscape-rust.md) — Rust peers (fjall, SurrealKV, redb, …)
- [`docs/compare-fjall.md`](docs/compare-fjall.md) — PedraDB vs fjall
- [`docs/robustness-vs-rocks-pebble-fdb.md`](docs/robustness-vs-rocks-pebble-fdb.md) — honest robustness vs RocksDB / Pebble / FDB (+ residual crash/bitrot matrix)
- [`docs/rocksdb-critiques-and-improvements.md`](docs/rocksdb-critiques-and-improvements.md) — detailed critiques
- [`docs/open-items.md`](docs/open-items.md) — living list of open items and status
- [`docs/references/`](docs/references/) — all primary sources (papers, docs)

License: Apache-2.0.
