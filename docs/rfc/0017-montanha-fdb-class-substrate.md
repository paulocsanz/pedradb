# RFC-0017: Montanha as FDB-class substrate (TiKV-scale engineering path)

**Status:** in-progress (P0–P2 **lab substrate** slices shipped; **not** FDB/TiKV field parity)  
**Updated:** 2026-08-13  
**Parent:** [RFC-0013](0013-montanhadb-product.md) (Montanha product)  
**Local kernel:** PedraDB — [RFC-0014](0014-rocks-pebble-redwood-maturity.md), [RFC-0016](0016-pedradb-production-robustness.md)  
**Alignment:** [`../montanha-vs-foundationdb.md`](../montanha-vs-foundationdb.md), [`../montanha-layering-dcs-on-store.md`](../montanha-layering-dcs-on-store.md), [`../fdb-limitations-analysis.md`](../fdb-limitations-analysis.md)  
**Peer / lab gates:** [RFC-0021](0021-montanha-fdb-tikv-parity-gaps.md).  
**Functional FDB parity + N-writer layers:** [RFC-0022](0022-montanha-fdb-functional-parity-and-layer-substrate.md). **0017 done ≠ FDB parity.**

---

## Background

**Product identity (locked):** Montanha is **FoundationDB-shaped** — ordered multi-key transactions as the substrate; DCS/SQL/streams/HA are **layers**, not a second etcd product.

**Engineering tools (not identity):** multi-Raft ranges, PD-like placement, and TiKV-like sharding are **how** we may scale writes — same way TiKV is not “the product of MySQL.” Confusing the two produces a TiKV clone without FDB’s layer story.

Today (honest):

| Capability | Montanha / store today | FDB | TiKV |
|------------|------------------------|-----|------|
| Local engine | PedraDB embed per node | Redwood / B-tree path | RocksDB |
| Multi-key TX | Local Pedra + store majority for ranges | System-wide MVCC | Percolator-class on Rocks |
| Multi-Raft ranges | Lab Queued RPC + failover MVP | Different unbundled log path | Production multi-Raft |
| Placement / rebalance | Minimal | Coordinators + data distributor culture | PD |
| Simulation | Kernel DST + World lab | Decades of Simulation / buggify | Jepsen + chaos, not FDB-scale |
| Multi-process / TCP prod | **P0.1 shipped** (MTCP wire + `montanha-tcp` + caixote lab) | Yes | Yes |
| Layers (DCS on store) | Yes (direction) | Yes | Separate products often |

**Why now:** kernel feature shape is catching up (0014/0015). Without a **cluster substrate plan**, Montanha stays a demo of multi-Raft on Pedra, not a peer of FDB/TiKV.

---

## Problems this solves

- **Problem:** No clear path from in-process Queued multi-Raft to multi-process production.  
- **Problem:** “FDB-like” claimed without simulation of *cluster* failures (partition, dual leader, clock skew).  
- **Problem:** Cross-range / multi-key distributed TX undefined or too weak for FDB-class apps.  
- **Problem:** Membership, rebalance, and snapshot catch-up are MVP-quality.  
- **Problem:** TiKV comparisons drag the product identity toward “another Rocks+Raft” instead of layers+TX.

---

## Proposed solution

### Product thesis (non-negotiable)

```text
Apps / SQL / DCS / HA agents
        │  TX + ordered keys only
        ▼
Montanha Store (ranges, majority, placement)
        │  embeds
        ▼
PedraDB per node (RFC-0016-hardened)
```

- **Do not** ship a second coordination database brand.  
- **Do** make DCS/SQL layers that cannot violate store majority.  
- **Do** document TiKV as *implementation cousin for sharding*, FDB as *product cousin for layers+TX*.

### Technical pillars

1. **Process model** — one storage process (or container) per disk domain; TCP (or equivalent) Raft; durable raft-meta on Env (already direction of 0015).  
2. **Range multi-Raft** — majority commit, install snapshot, membership change under partitions.  
3. **Distributed TX** — start with **single-range multi-key** (strong); add **2PC / parallel commit–class** cross-range only when P1 gates pass (TiKV/Percolator lessons, FDB-style client libraries later).  
4. **Cluster simulation** — deterministic schedule of Net+Disk+Clock+membership; seed-replay; silent_wrong=0 for store invariants (I-MAJ, I-RD, I-HA, I-DCS).  
5. **Placement** — simple PD-like assigner (even single binary) before fancy auto-rebalance.  
6. **Depend on Pedra 0016** — no Montanha GA if local engine fails fence/vlog-GC/soak gates that Montanha enables by default.

---

## Delivery slices

### P0 — multi-process majority that is not a toy

Ship a path operators can run on 3 machines/VMs with real sockets and survive leader kill.

- [x] **P0.1** Production-shaped transport: multi-process Raft over TCP with Env-backed raft persist — status: `done` (`tcp.rs` MTCP frames, `open_single_node`, `montanha-tcp` binary, `tests/tcp_multihost.rs`, caixote `Dockerfile.montanha-tcp` + `caixote.config.ts`)  
- [x] **P0.2** 3-node elect + put + leader kill + majority read invariants automated — status: `done` (in-process Queued + `rfc20_*` + multi-process smoke + TCP multi-port majority)  
- [x] **P0.3** InstallSnapshot / catch-up after long partition for one lagging peer — status: `done` (lagging partition heal + membership re-add catch-up tests in `montanha_fdb_path`)  
- [x] **P0.4** Cluster DST skeleton: inject message drop + seed-replay; I-MAJ holds — status: `done` (`cluster_dst_lossy_net_i_maj_holds`, `cluster_dst_seed_replay_*`)  

### P1 — scale-out substrate (TiKV-class engineering, FDB-class product)

- [x] **P1.1** Multi-range multiwrite + routing helper (`put_routed`) + multi-process multiwrite smoke — status: `done` (dynamic split still open)  
- [x] **P1.2** Cross-range TX: `commit_tx` shipped + multi-process verify; single-range `put_batch` fast path — status: `done`  
- [x] **P1.3** Membership change (add/remove voter) tests — status: `done` (`membership_remove_add_catchup`)  
- [x] **P1.4** Store chaos soak script + FDB-path suite — status: `done` (`scripts/montanha_chaos_soak.sh`)  

### P2 — FDB-peer trajectory (not day-one clone)

- [x] **P2.1** Stronger simulation: clock skew, disk full on majority, rolling restart — status: `done` (`p21_rolling_restart_*`, `p21_clock_skew_*`, `p21_disk_full_on_majority_*` in `montanha_fdb_path`)  
- [x] **P2.2** Client library contract (retry, error classes, not_leader) documented + tests — status: `done` (`client.rs` `ClientClass` / `TcpClusterClient`, `tcp_client_retry_not_leader`, `docs/montanha-client-contract.md`)  
- [x] **P2.3** Layer freeze: DCS + one app layer proven on multi-process store only (no dual path) — status: `done` (`dcs-layer` smoke + `multi_process_dcs_layer_freeze`, `docs/montanha-layer-freeze.md`)  
- [x] **P2.4** Honest comparison doc update (vs FDB / TiKV) with measured limits — status: `done` (`montanha-vs-foundationdb.md` §5.5)  

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Multi-process Raft transport + Env persist | done | montanha-tcp + tcp_multihost + caixote | 2026-08-13 |
| P0.2 | p0 | 3-node kill-leader majority tests | done | rfc20 + montanha_fdb_path | 2026-08-13 |
| P0.3 | p0 | Snapshot catch-up lagging peer | done | lagging_partition_heals | 2026-08-13 |
| P0.4 | p0 | Cluster DST skeleton seed-replay | done | lossy net + seed replay | 2026-08-13 |
| P1.1 | p1 | Multi-range multiwrite + routing | done | put_routed + multiwrite tests | 2026-08-13 |
| P1.2 | p1 | Cross-range TX policy shipped | done | commit_tx + mp smoke | 2026-08-13 |
| P1.3 | p1 | Membership under load | done | remove/add catchup | 2026-08-13 |
| P1.4 | p1 | Store soak + chaos nightly | done | montanha_chaos_soak.sh | 2026-08-13 |
| P2.1 | p2 | Richer cluster simulation | done | p21_* montanha_fdb_path | 2026-08-13 |
| P2.2 | p2 | Client error/retry contract | done | client.rs + tcp_client_retry | 2026-08-13 |
| P2.3 | p2 | DCS-only on multi-process store | done | dcs-layer freeze + layer-freeze.md | 2026-08-13 |
| P2.4 | p2 | Comparison doc with numbers | done | montanha-vs-foundationdb §5.5 | 2026-08-13 |

---

## Acceptance criteria

### Tests

**P0**

- [x] Multi-process write/verify smoke + in-process kill-leader / majority + **TCP 3-process elect/put/majority** (`montanha-tcp`, `tcp_multihost`).  
- [x] Partitioned lagging peer heal + catch-up progress; membership re-add.  
- [x] DST seed S: same seed → same leader+maj; I-MAJ under lossy Queued net.

**P1**

- [x] ≥2 ranges; multiwrite to different leaders; `put_routed` + fast RO.  
- [x] Cross-range: `commit_tx` atomic multi-process path.  
- [x] Add/remove voter membership tests.

**P2**

- [x] Chaos suite green for fixed seeds list.  
- [x] Client contract tests for `NotLeader` / retry.  
- [x] DCS create/cas only through multi-process store path in CI.

### Telemetry / analytics

- Per-range leader, commit index, lag, snapshot installs.  
- DST: seeds run, failures injected, invariants checked (CI artifact or log).

### Documentation

- This RFC status table.  
- Update [`montanha-vs-foundationdb.md`](../montanha-vs-foundationdb.md) when P0 lands (multi-process real).  
- Layering doc: ban “use etcd for prod, Montanha for demo.”  
- Depend on Pedra [RFC-0016](0016-pedradb-production-robustness.md) P0 for any GA that enables large values or heavy write amp.

### Screenshots

- backend-only (optional topology diagram in docs later).

---

## Out of scope

- Cloning FDB’s exact role split (proxies/resolvers/logs) on day one.  
- Full Spanner/TrueTime.  
- Replacing PedraDB with RocksDB under Montanha (kernel ownership is the bet).  
- SQL completeness or etcd API parity as the product identity.  
- Claiming FDB field parity without simulation hours + production time.

---

## TiKV vs FDB — how this RFC uses both

| Steal from | What | Do not steal |
|------------|------|--------------|
| **FDB** | Layers + TX core; simulation culture; “one substrate” | Blind copy of 5s TX limits / internal C++ stack |
| **TiKV** | Multi-Raft range ops lore; PD placement ideas; snapshot catch-up practice | Product identity as “MySQL-compatible via TiDB only”; Rocks as destiny |
| **etcd** | Majority + watch *need* | Shipping etcd as required dependency for Montanha |

---

## Dependency on Pedra (RFC-0016)

| Montanha wants… | Pedra must have… |
|-----------------|------------------|
| Large values in ranges | Vlog **GC** (0016 P0.1) or threshold off |
| High write QPS on leader | Group commit / less fsync serialization (0016 P1.1) |
| Long soak | Kernel silent_wrong gate (0016 P0.4) |
| Trust under ENOSPC | Fence + Env already (0015); denser schedules (0016 P0.4) |

Montanha P0 can proceed on current Pedra for **metadata-sized** keys; **do not** default large_value_threshold in store nodes until 0016 P0.1 is done.

---

## Success definition (“peer”)

| Bar | Meaning |
|-----|---------|
| **Peer of FDB (product)** | Serious apps build layers only on Montanha TX; no shadow etcd |
| **Peer of TiKV (engineering)** | Multi-Raft ranges, rebalance, catch-up, chaos survive |
| **Not yet peer of either (field)** | Requires years of production — this RFC only reaches *launchable substrate* |

**Launchable substrate** = P0+P1 green + Pedra 0016 P0 green + honest docs. **Field peer** is out of any single RFC.
