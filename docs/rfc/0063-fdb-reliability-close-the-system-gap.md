# RFC: 0063 — Fechar o gap de *sistema* vs FDB (lab=produto, volume REAL, joint)

**Status:** in-progress
**Updated:** 2026-08-26
**Parents:** [0050](0050-world-in-tree-fdb-determinism.md), [0051](0051-beyond-fdb-sim-holes.md), [0053](0053-ironfleet-years.md), [0059](0059-massive-scale-parallel-dst-and-cluster-invariants.md), [0061](0061-residuals-sel4-ironfleet.md)

## Background

- FDB Sim2 **é** o `fdbserver` (Flow). Pedra World é Queued + `InProcessNet`; `RpcMode::Direct` é pump síncrono do **mesmo** `PeerMsg` (RFC-0050 P1.1) — não um segundo codec, mas um segundo *delivery* que o seed não vê (sem drop/reorder). `montanha-tcp` ainda faz `thread::spawn` + relógio de parede. io_uring está fora do perfil verificado por contrato (sem modelo de ring). `db.rs` glue não se extrai (L46).
- O FDB **também** tem dual-path: G1 cliente/threads, G3 FS, G4 TCP, G5 Rocks/gRPC, G6 S3, G7 operador. Eles reescreveram o *servidor* em Flow; o resto ficou tester não-replayável + campo. Nós não vamos reescrever em Flow (L29).
- Membership out-of-band + quorum floor (seed 500308) recusa shrink unsound. Não é joint consensus.
- L28 MEASURE: swarm in-process ≠ cluster multi-processo reproduzido por seed.
- Composição vote∧AE∧commit∧apply já existe (`compose_model.rs`); iff do voto já é twin Verus. O que faltava era *sistema*: config no log, hunter de seeds, crash flush+tail.

## Problems This Solves

- **Problem:** “lab ≠ produto” parece um defeito nosso; o mapa G1–G8 mostra o que o FDB também não mete no Sim2.
- **Problem:** floor recusa rollouts profundos; falta o path log-carried.
- **Problem:** volume sem dente = soak; hunter UCB1 escolhe seeds que exercitam membership/disk.

## Proposed Solution

P0 útil sozinho: (1) residual Direct nomeado; (2) `MembershipJoint` no log com quorum old∧new; (3) crash-dictionary flush+tail no World; (4) `world_hunt` UCB1; (5) `multiproc_trace_smoke` continua o dente REAL de *sim* multi-proc (não TCP cluster — L28 ainda MEASURE).

## Delivery slices (mandatory)

### P0 — must ship first
- [x] **P0.1** Residual `R-direct-rpc` (Direct = mesmo PeerMsg, delivery sem Net) — status: `done`
- [x] **P0.2** `RangeEntry::MembershipJoint` + `remove_member_joint` (quorum old∧new; floor continua no out-of-band) — status: `done` (`log_carried_joint_remove_crosses_out_of_band_floor`, `world_joint_remove_is_queued_and_replayable`)
- [x] **P0.3** Crash flush+tail no World (`world_flush_then_tail_put_survives_crash_reopen`) — status: `done`
- [x] **P0.4** `world_hunt` UCB1 (reward membership/disk/silent_wrong) — status: `done`
- [x] **P0.5** Composição kernels: já shipped 0053 (`compose_model` + `tcp_node_model`); este RFC não duplica — status: `done`

### P1 — next wave
- [ ] **P1.1** Joint nas *eleições* (hoje só commit); union old∪new no RequestVote — status: `todo`
- [ ] **P1.2** TCP produção no mesmo pump Queued (fingerprint `PeerMsg` bytes no `montanha-tcp`) — status: `todo`
- [ ] **P1.3** L28 REAL: 1 bug de cluster multi-proc TCP pinado a seed S — status: `todo` (MEASURE até repro)

### P2 — later
- [ ] **P2.1** Kernel Verus do joint (não alargar HTTP) — status: `todo`
- [ ] **P2.2** PCT d>2 campaign (0051 runner já aceita `depth`) — status: `todo`
- [ ] **P2.3** TCG guest iff `PEDRA_QEMU_SSH` (não inventar guest) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Residual Direct RPC | done | R-direct-rpc | 2026-08-26 |
| P0.2 | p0 | MembershipJoint log-carried | done | store+world tests | 2026-08-26 |
| P0.3 | p0 | Crash flush+tail World | done | world_flush_then_tail… | 2026-08-26 |
| P0.4 | p0 | world_hunt UCB1 | done | bin world_hunt | 2026-08-26 |
| P0.5 | p0 | compose_model already 0053 | done | no duplicate | 2026-08-26 |
| P1.1 | p1 | Joint elections | todo | — | 2026-08-26 |
| P1.2 | p1 | TCP=Queued PeerMsg | todo | — | 2026-08-26 |
| P1.3 | p1 | L28 REAL TCP cluster | todo | MEASURE | 2026-08-26 |
| P2.1 | p2 | Verus joint kernel | todo | — | 2026-08-26 |
| P2.2 | p2 | PCT d>2 campaign | todo | — | 2026-08-26 |
| P2.3 | p2 | TCG guest iff SSH | todo | — | 2026-08-26 |

## Acceptance Criteria

- **Tests:** `log_carried_joint_remove_crosses_out_of_band_floor`; `world_joint_remove_is_queued_and_replayable`; `world_flush_then_tail_put_survives_crash_reopen`; `world_committed_put_visible_or_fail_closed` (crash+reopen).
- **Telemetry / Analytics:** `world_hunt` JSONL; sem claim CPU-hours vs Apple.
- **Documentation:** este RFC; `R-joint` passa a `open`; `R-direct-rpc` contínuo.
- **Screenshots:** backend-only.

## Out of scope

- Extrair `db.rs` (L46). Flow (L29). Inventar guest TCG. Coluna B Rocks.
