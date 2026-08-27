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

**Encrypt-at-rest:** the engine does not implement it. Use LUKS / FileVault / volume encryption. Never a claim of LSM-level cipher.

## Status (shipped)

| Layer | Crates | Status |
|-------|--------|--------|
| Kernel LSM + TX | `pedradb-core` | ✅ WAL, MemTable, **SST v4 + Bloom + lz4**, leveled compact, MANIFEST v2, LOCK, checkpoint/stats/verify, scan, ConcurrentDb, range delete |
| Fault injection | `pedradb-sim` | ✅ FailingEnv, RecordingEnv, lying sync, short-write, seed, Arc |
| Oracle | `pedradb-oracle` | ✅ model (+ optional RocksDB) |
| Ordered apply | `pedradb-apply` | ✅ LogApplier, FakeLog, KvService, InProcessCluster |
| Raft (TCP + persist) | `pedradb-raft`, `pedra-raft-node` | ✅ elect, put, multi-process, failover |
| **Montanha-Store** multi-Raft | `pedradb-store`, `montanha-tcp` | ✅ ranges + put/get + DCS + Queued RPC + **TCP multi-host (MTCP)** |
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
| Montanha FDB-class substrate | [RFC-0017](docs/rfc/0017-montanha-fdb-class-substrate.md) | P0 done (incl. **TCP multi-host P0.1** + caixote Linux lab); P2 open |
| L1 primitive for platform + Scylla-need | [RFC-0019](docs/rfc/0019-local-primitive-for-platform-and-scylla-need.md) | done (P0–P2.2) — CAS, seq pin, change feed, multi_get, soak, compact_for_reads |
| Synthetic field maturity | [RFC-0020](docs/rfc/0020-synthetic-field-maturity.md) | **P0–P2 done** — gate, soaks, canaries (lease/index/journal), cluster matrix, explore, race, fuzz, residuals |
| World in-tree (FDB-shaped determinism) | [RFC-0050](docs/rfc/0050-world-in-tree-fdb-determinism.md) | **done (P0–P2)** — `crates/pedradb-world`: seed→`trace_hash` + invariantes + seam inventory no CI; lab=produto (PeerMsg, canaries, buggify); P2: π ordena `World::run` (5º seam), swarm L28 24/24 clean (gate REAL aberto), papel fold no mesmo seed, det_io hard-CT com sibling. Not FDB Simulation |
| Beyond Sim2 holes (π / layers / OS) | [RFC-0051](docs/rfc/0051-beyond-fdb-sim-holes.md) | **done (P0–P2)** — PCT sobre `ConcurrentDb` real: plantado d=2 3/256 (seq 0), fence de grupo no fsync off-lock 9/256 (seq 0), OCC plantado 38/256 cross-group / correto 0/256 (oráculo group-aware), CommitUnknown canário, trial `FailingEnvArc<IoUringEnv>` Linux, guards spawn/wall-clock no CI. Not “more trusted than FDB” |
| Intensidade máxima (paralelo + caixas + formal) | [RFC-0057](docs/rfc/0057-maximum-intensity-parallel-dst-boxes-formal.md) | **draft (P0.1+P0.2 done)** — swarm DST multi-núcleo com forenses por-run e trials paralelos não-interferentes; caixas (Miri/TSan/ASan) como jobs irmãos no CI; kernel formal do group-commit + OCC. “100%” = espaço verificável relativo ao TCB; residual publicado |
| Modo verificado (fallback dos kernels) | [RFC-0058](docs/rfc/0058-verified-mode-kernel-derived-fallback.md) | **P0+P1+P2 done** — perfil de produto com seções críticas = kernels provados (`OpenOptions::verified()`; group-commit de volta por teorema: `group_commit_kernel.rs`, Verus 14/0 + Lean no-sorry, merge ativo com catch-up 0 e bypass async — `verified-v2`; `profile_report()` amarrado ao `catalog.json` machine-checked); suíte FailingEnv + World + PCT rodando no perfil (silent_wrong=0, fence ≤ 1 escritor); `open_verified` em fold/lease/dcs/compat (StdEnv por tipo); derivação de semântica verified=full nos oráculos; CI `verified-mode`; ring io_uring **fora** do modo com contrato publicado (P2.2); `PEDRA_VERIFIED=1` no CLI como linha de produto (P2.3). Extração total segue REFUSE (VeriBetrKV 8×) |
| Escala massiva paralela + invariantes de cluster | [RFC-0059](docs/rfc/0059-massive-scale-parallel-dst-and-cluster-invariants.md) | **draft (P0 done)** — `world_swarm` (work-stealing por núcleo, backend mem com mesmas seams de falha, gate serial-vs-paralelo por `trace_hash`); invariant checker cross-node no estado convergido (autenticidade/split-brain/ressurreição); escala 7/9 nós; 4 F-found corrigidos com regressão pinada (CRC do frame, discard escapado, CommitUnknown por payload, snapshot stale wipe). Campanhas: 16384@3n/4096@7n/4096@9n-4ranges — sem claim CPU-hours vs FDB |
| DST inside boxes (Miri / ASan / TCG) | [RFC-0052](docs/rfc/0052-dst-inside-boxes.md) | **draft** — compose in the cycle, not one VM. P0: `FailingEnv` under Miri. REFUSE Miri-in-TCG and TCG benches |
| IronFleet-scale formal (years) | [RFC-0053](docs/rfc/0053-ironfleet-years.md) | **done (Y1–Y3)** — Lean AE/commit extracts; AE/apply caller refinements; reopen kernel + lemmas; bounded liveness sob axioma quórum-vivo; π/VerusSync não disparado (RFC-0051 draft) |
| Entregar o 100% (relativo ao TCB) | [RFC-0056](docs/rfc/0056-one-hundred-percent-delivery.md) | **done** — itens 1–6/8/9/11 do checklist verdes; 7 gated (RFC-0051 PCT); 10 “TCB à vista” (freeze no CI); 12 contínuo. Estado: [one-hundred-percent-report](docs/formal/one-hundred-percent-report.md) |
| Lease / index / journal canaries | `pedradb-lease`, `pedradb-index`, `pedradb-journal` | ✅ W1–W4 workloads silent_wrong=0 |

RFCs **0001–0012**, **0014**, **0015**, **0019** delivered. **0020** is the confidence/volume program; **0016/0017** remain robustness + cluster substrate (not Rocks field parity claims).

## Build & test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo bench -p pedradb-core --bench baseline
cargo run -p pedradb-cli -- demo /tmp/pedra-demo
# Montanha TCP multi-host (RFC-0017 P0.1) — 3 processes, elect+put+majority
cargo test -p pedradb-store --test tcp_multihost
cargo run -p pedradb-store --bin montanha-tcp -- node --id 1 --data /tmp/m1 --bind 127.0.0.1:9701 \
  --peer 1=127.0.0.1:9701 --peer 2=127.0.0.1:9702 --peer 3=127.0.0.1:9703
# Real Linux lab on caixote (3-process local3; smoke on start + external ports)
./scripts/montanha_tcp_caixote.sh all
# Parallel World swarm campaign (RFC-0059): n_seeds start_seed workers n_nodes steps
#   env: PEDRA_SWARM_RANGES=4 (multi-range) | PEDRA_SWARM_DISK=1 (real-FS nodes)
#        PEDRA_SWARM_BUGGIFY=0 (faults off) | PEDRA_SWARM_CONSISTENCY=0 (checker off)
#        PEDRA_SWARM_LOG=dir (JSONL per seed + summary) | PEDRA_SWARM_DUMP=<seed>
#        (single-seed diagnostic: event trace; PEDRA_SWARM_TRACE=1 prints every event)
# exit 1 on any oracle failure (silent_wrong / row_half / cross-node consistency)
cargo run --release -p pedradb-world --bin world_swarm -- 16384 1 0 3 12
# docker smoke: docker run --rm --platform linux/amd64 -e CLUSTER=smoke ghcr.io/paulocsanz/montanha-tcp:p01-m3
# Multi-container mesh (experimental): MONTANHA_MESH=1 — blocked on caixote WG/hairpin today
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
