# Status ledger — crates, RFCs, and reproduction commands

Detailed per-crate shipping status and the RFC program. Moved here from the
README front page; this is the living ledger, the README is the pitch.

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
| Toward Rocks/Pebble/Redwood | [RFC-0014](rfc/0014-rocks-pebble-redwood-maturity.md) | done P0–P2 (OCC, vlog spill, incremental backup) |
| Audit durability / Env fixes | [RFC-0015](rfc/0015-audit-pedradb-correctness-fixes.md) | done (P0–P2) |
| Pedra production robustness | [RFC-0016](rfc/0016-pedradb-production-robustness.md) | P0 done — `compact_vlog`, stats, soak; P1 group-commit done; P1.4/P2.1 open |
| Montanha FDB-class substrate | [RFC-0017](rfc/0017-montanha-fdb-class-substrate.md) | P0 done (incl. **TCP multi-host P0.1** + caixote Linux lab); P2 open |
| L1 primitive for platform + Scylla-need | [RFC-0019](rfc/0019-local-primitive-for-platform-and-scylla-need.md) | done (P0–P2.2) — CAS, seq pin, change feed, multi_get, soak, compact_for_reads |
| Synthetic field maturity | [RFC-0020](rfc/0020-synthetic-field-maturity.md) | **P0–P2 done** — gate, soaks, canaries (lease/index/journal), cluster matrix, explore, race, fuzz, residuals |
| World in-tree (FDB-shaped determinism) | [RFC-0050](rfc/0050-world-in-tree-fdb-determinism.md) | **done (P0–P2)** — `crates/pedradb-world`: seed→`trace_hash` + invariantes + seam inventory no CI; lab=produto (PeerMsg, canaries, buggify); P2: π ordena `World::run` (5º seam), swarm L28 24/24 clean (gate REAL aberto), papel fold no mesmo seed, det_io hard-CT com sibling. Not FDB Simulation |
| Beyond Sim2 holes (π / layers / OS) | [RFC-0051](rfc/0051-beyond-fdb-sim-holes.md) | **done (P0–P2)** — PCT sobre `ConcurrentDb` real: plantado d=2 3/256 (seq 0), fence de grupo no fsync off-lock 9/256 (seq 0), OCC plantado 38/256 cross-group / correto 0/256 (oráculo group-aware), CommitUnknown canário, trial `FailingEnvArc<IoUringEnv>` Linux, guards spawn/wall-clock no CI. Not “more trusted than FDB” |
| Intensidade máxima (paralelo + caixas + formal) | [RFC-0057](rfc/0057-maximum-intensity-parallel-dst-boxes-formal.md) | **draft (P0.1+P0.2 done)** — swarm DST multi-núcleo com forenses por-run e trials paralelos não-interferentes; caixas (Miri/TSan/ASan) como jobs irmãos no CI; kernel formal do group-commit + OCC. “100%” = espaço verificável relativo ao TCB; residual publicado |
| Modo verificado (fallback dos kernels) | [RFC-0058](rfc/0058-verified-mode-kernel-derived-fallback.md) | **P0+P1+P2 done** — perfil de produto com seções críticas = kernels provados (`OpenOptions::verified()`; group-commit de volta por teorema: `group_commit_kernel.rs`, Verus 14/0 + Lean no-sorry, merge ativo com catch-up 0 e bypass async — `verified-v2`; `profile_report()` amarrado ao `catalog.json` machine-checked); suíte FailingEnv + World + PCT rodando no perfil (silent_wrong=0, fence ≤ 1 escritor); `open_verified` em fold/lease/dcs/compat (StdEnv por tipo); derivação de semântica verified=full nos oráculos; CI `verified-mode`; ring io_uring **fora** do modo com contrato publicado (P2.2); `PEDRA_VERIFIED=1` no CLI como linha de produto (P2.3). Extração total segue REFUSE (VeriBetrKV 8×) |
| Escala massiva paralela + invariantes de cluster | [RFC-0059](rfc/0059-massive-scale-parallel-dst-and-cluster-invariants.md) | **draft (P0 done)** — `world_swarm` (work-stealing por núcleo, backend mem com mesmas seams de falha, gate serial-vs-paralelo por `trace_hash`); invariant checker cross-node no estado convergido (autenticidade/split-brain/ressurreição); escala 7/9 nós; 4 F-found corrigidos com regressão pinada (CRC do frame, discard escapado, CommitUnknown por payload, snapshot stale wipe). Campanhas: 16384@3n/4096@7n/4096@9n-4ranges — sem claim CPU-hours vs FDB |
| DST inside boxes (Miri / ASan / TCG) | [RFC-0052](rfc/0052-dst-inside-boxes.md) | **draft** — compose in the cycle, not one VM. P0: `FailingEnv` under Miri. REFUSE Miri-in-TCG and TCG benches |
| IronFleet-scale formal (years) | [RFC-0053](rfc/0053-ironfleet-years.md) | **done (Y1–Y3)** — Lean AE/commit extracts; AE/apply caller refinements; reopen kernel + lemmas; bounded liveness sob axioma quórum-vivo; π/VerusSync não disparado (RFC-0051 draft) |
| Entregar o 100% (relativo ao TCB) | [RFC-0056](rfc/0056-one-hundred-percent-delivery.md) | **done** — itens 1–6/8/9/11 do checklist verdes; 7 gated (RFC-0051 PCT); 10 “TCB à vista” (freeze no CI); 12 contínuo. Estado: [one-hundred-percent-report](formal/one-hundred-percent-report.md) |
| Lease / index / journal canaries | `pedradb-lease`, `pedradb-index`, `pedradb-journal` | ✅ W1–W4 workloads silent_wrong=0 |
| Slipstream scale ≥2× Rocks default (1M–100M) | [RFC-0160](rfc/0160-slipstream-scale-2x.md) | **draft** — required set hydrate/settle/get_hit/prefix/lookup at 1M/10M/25M/100M; P0 = 100M runs + named lookup hole; P1 = ≥1× all cells; P2 = ≥2×. Peer Rocks `sync=false`, guest `linux-gate-p149b` only |

RFCs **0001–0012**, **0014**, **0015**, **0019** delivered. **0020** is the confidence/volume program; **0016/0017** remain robustness + cluster substrate (not Rocks field parity claims).

## RFC index (entry points)

[`rfc/`](rfc/) — start with
[0001](rfc/0001-pedradb-high-level-spec.md) (spec), engine
[0009](rfc/0009-rocksdb-class-engine.md), maturity wave 2
[0014](rfc/0014-rocks-pebble-redwood-maturity.md), audit
[0015](rfc/0015-audit-pedradb-correctness-fixes.md), production robustness
[0016](rfc/0016-pedradb-production-robustness.md), Montanha FDB-class
[0017](rfc/0017-montanha-fdb-class-substrate.md), products
[0010](rfc/0010-dbs-on-top.md), faults
[0011](rfc/0011-env-fault-injection.md), multi-node/wire
[0012](rfc/0012-next-significant-steps.md).

## Reproduce / advanced commands

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
