# RFC-0021: Montanha → FDB/TiKV parity gaps (honest peer trajectory)

**Status:** in-progress (all Status table rows `done` as lab gates; **still not** FDB field peer)  
**Updated:** 2026-08-13  
**Parent:** [RFC-0017](0017-montanha-fdb-class-substrate.md) (lab substrate — **not** field peer)  
**Alignment:** [`../montanha-vs-foundationdb.md`](../montanha-vs-foundationdb.md) §5.5  
**Kernel:** PedraDB · [RFC-0016](0016-pedradb-production-robustness.md) · [RFC-0018](0018-fdb-method-parity-and-fault-coverage.md)

---

## 0. Honesty contract (non-negotiable)

| Claim that is **false** | Truth |
|-------------------------|--------|
| “Montanha is FDB-class / field peer” | **False.** Gates and product surface advanced; field pedigree and full TX isolation remain open. |
| “RFC-0017 done ⇒ parity” | 0017 = lab multi-Raft MVP. Parity program = **this RFC**. |
| “P0.4 done ⇒ 8h always ran in CI” | Gate **supports** 8h via `PEDRA_SIM_VOLUME_WALL_SECS=28800`; CI runs a dense short bank. |
| “P2.3 done ⇒ TLS on wire” | rustls **lab** behind `--features tls` (RFC-0050); cleartext still the default. Not GA. |

**Definition of field peer (still unmet):** FDB-feel TX (OCC/snapshot) + multi-region ops maturity + years of production evidence.

---

## Background

0017 shipped multi-process/TCP majority, DST skeleton, DCS freeze, caixote mesh.  
0021 ships the **parity workstreams** so status cannot hide open gaps.

---

## Problems this solves

- Apps speak Raft `put` → need TX product API.  
- Commit path unmeasured → need perf artifacts.  
- Sim too narrow → need seed-bank volume gate.  
- Ops folklore → need status JSON / split / drills.  
- Architecture cosplay → need numbers-backed commit-path decision.

---

## Proposed solution

Five workstreams with **executable gates** (scripts + tests), not slides.

```text
W1 Client TX     W2 Commit scale     W3 Sim@volume
W4 Control/ops   W5 Perf ceilings
```

---

## Delivery slices

### P0

- [x] **P0.1** Client TX `PendingTx` + TCP `CommitTx` — status: `done`  
- [x] **P0.2** Typed limits/errors + `ClientClass::LimitRejected` — status: `done`  
- [x] **P0.3** Perf gate v0 (`montanha-perf-gate`, `scripts/montanha_perf_gate_v0.sh` → `findings/perf-*/perf_report.json`) — status: `done`  
- [x] **P0.4** Sim volume v0 (`scripts/montanha_sim_volume_v0.sh`, 50-seed bank, wall budget for 8h) — status: `done`  
- [x] **P0.5** Cluster status JSON (`cluster_status_json`, HTTP `/v1/cluster`) — status: `done`  

### P1

- [x] **P1.1** Cross-range TX hardened proofs (`commit_tx_finish_fail_*`, multiproc write/verify) — status: `done`  
- [x] **P1.2** Dynamic range split minimal (`split_range_at`) — status: `done`  
- [x] **P1.3** Membership/dial without host SSH — status: `done` (`set_peer_addrs`, `TcpClusterClient::rewire_peer_map` via TCP SetPeers)  
- [x] **P1.4** Fault injection on real TCP + multi-fault suite (`montanha_tcp_fault_inject.sh`) — status: `done`  
- [x] **P1.5** Backup/restore drill (`montanha_backup_restore_drill.sh`) — status: `done`  
- [x] **P1.6** Perf soak knobs (`montanha_perf_soak_v0.sh`) — status: `done`  

### P2

- [x] **P2.1** 24h sim entrypoint (`montanha_sim_24h.sh` → volume gate wall) — status: `done` (entrypoint; not that 24h was run in every CI)  
- [x] **P2.2** Commit-path scale decision — status: `done` ([0021-commit-path-scale-decision.md](0021-commit-path-scale-decision.md): default multi-Raft ranges)  
- [x] **P2.3** Security TLS **baseline doc** — status: `done` (RFC-0050: rustls lab + `--require-tls`; cleartext still default)  
- [x] **P2.4** Rolling upgrade + multi-AZ runbook — status: `done`  
- [x] **P2.5** YCSB-class v0 script — status: `done` (`montanha_ycsb_class_v0.sh`)  
- [x] **P2.6** Multi-region **lab** (region tags + prefer-region dial) — status: `done` ([0021-geo-multiregion.md](0021-geo-multiregion.md); **not** geo-HA field)  

---

## Status (living)

| ID | Band | Title | Status | Evidence | Updated |
|----|------|-------|--------|----------|---------|
| P0.1 | p0 | Client TX surface | done | `PendingTx`, TCP CommitTx, tests | 2026-08-13 |
| P0.2 | p0 | Error & limit contract | done | `ValueTooLarge`/`TransactionTooLarge`/`LimitRejected` | 2026-08-13 |
| P0.3 | p0 | Perf gate v0 | done | `montanha-perf-gate`, `findings/perf-gate-v0-verify/` | 2026-08-13 |
| P0.4 | p0 | Sim volume v0 | done | `montanha_sim_volume_v0.sh`, `findings/sim-volume-v0-verify/` | 2026-08-13 |
| P0.5 | p0 | Cluster status API | done | `cluster_status_json`, `/v1/cluster` | 2026-08-13 |
| P1.1 | p1 | Cross-range TX hardened | done | `commit_tx_finish_fail_*`, multiproc | 2026-08-13 |
| P1.2 | p1 | Dynamic range split | done | `split_range_at` + test | 2026-08-13 |
| P1.3 | p1 | Membership without host set-peers | done | `set_peer_addrs`, `rewire_peer_map`, `tcp_rewire_peer_map_without_ssh` | 2026-08-13 |
| P1.4 | p1 | TCP fault injection | done | `montanha_tcp_fault_inject.sh` | 2026-08-13 |
| P1.5 | p1 | Backup/restore drill | done | `montanha_backup_restore_drill.sh` | 2026-08-13 |
| P1.6 | p1 | Perf soak | done | `montanha_perf_soak_v0.sh` | 2026-08-13 |
| P2.1 | p2 | Sim 24h culture entry | done | `montanha_sim_24h.sh` | 2026-08-13 |
| P2.2 | p2 | Commit path decision | done | decision md: prefer multi-Raft scale | 2026-08-13 |
| P2.3 | p2 | TLS baseline | done | doc only | 2026-08-13 |
| P2.4 | p2 | Rolling upgrade runbook | done | runbook md | 2026-08-13 |
| P2.5 | p2 | YCSB-class v0 | done | `montanha_ycsb_class_v0.sh` | 2026-08-13 |
| P2.6 | p2 | Multi-region lab | done | `set_node_region`, client prefer-region, tests | 2026-08-13 |

---

## Deep dive (remaining true gaps vs FDB)

### Still not FDB-peer (honest residuals — not open Status checkboxes)

1. **OCC/snapshot TX** — PendingTx is buffer+atomic commit, not FDB read-version isolation.  
2. **TLS on wire** — baseline written, not default code.  
3. **Field pedigree** — no multi-year production.  
4. **Geo-HA / multi-region majority** — lab region tags only (P2.6), not DR.  
5. **Unbundled commit roles** — deliberately deferred (P2.2 chose multi-Raft first).

### Shipped levers

- Perf JSON with put/get/tx p50/p99 (honest: in-process, debug builds are slow).  
- Sim volume seed bank + chaos.  
- `/v1/cluster` JSON including regions/peers.  
- `split_range_at`, TCP rewire without SSH, region-aware dial.

---

## Acceptance criteria

### Tests / scripts (shipped)

- [x] PendingTx + limits + conflict unit tests.  
- [x] TCP CommitTx multi-node.  
- [x] Multiproc commit_tx write/verify.  
- [x] `cluster_status_json_has_leaders`.  
- [x] `split_range_at_two_ranges_put`.  
- [x] Perf gate produces `perf_report.json`.  
- [x] Sim volume produces `sim_volume_report.json` with silent_wrong=0 on short bank.  

### Documentation

- This Status table.  
- 0017 / open-items / comparison link 0021.  
- Commit-path decision, TLS baseline, upgrade runbook, geo deferral.

### Screenshots

- backend-only.

---

## Out of scope

- Claiming field parity with FoundationDB.  
- Claiming FDB field peer or Montanha GA (TLS lab ≠ production mesh).  
- Geo HA.  
- Softening Status to look greener than evidence.

---

## Relationship to RFC-0017

0017 = substrate lab. 0021 = parity program.  
**0017 smokes do not flip 0021 rows** without matching evidence.
