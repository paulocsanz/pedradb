# RFC-0181 — Follower leftover não pende no último put

**Status:** in-progress
**Updated:** 2026-09-07
**ID:** 0181
**Parents:** [0180](0180-overwrite-mc4-gt1x.md) P0.31 / P0.36
**Peer:** RocksDB default `WriteOptions.sync=false`. G1 não é win.

> P0 é local (teste que falha se o join pender). Caixa 4 GiB é P1.

## Background

- RFC-0180 P0.31: `lead` commita **um** grupo e resigna. O `put()` do
  leader deixa de incluir dezenas de grupos (p999 1,5 ms → ~80–100 µs).
- Quem chega **durante** o WAL `write()` fica em `queue.pending` com
  `FollowerReply` à espera. O próximo `put()` vira leader e leva-os.
  Isso é o caminho feliz (avg_group ~2,5 no overwrite_mc4).
- P0.36 drenou leftover no mesmo `lead()`: p48 avg 2,51→1,95,
  overwrite **0,87×**. Revertido. Canário `rfc0180_leftover_*` (8×32)
  junta; 400 k ops no DIAG não pendurou.
- Hang residual: último wave — 4 clientes, grupo de 3, 1 WAL-late, os
  3 já acabaram os ops. O leftover está em `recv()`; ninguém mais dá
  put; `PendingWrite` não tem `Drop` que complete. `DB` drop é depois
  do `join`. O canário não é o último-op.

## Problems This Solves

- **Problem:** último put de um follower pode `recv()` para sempre.
- **Problem:** drenar leftover no `lead()` parte grupos e mata QPS
  (medido). O hang não se fecha com o mesmo truque.

## Proposed Solution

Não encadear grupos no leader. No resign, se `pending` não está vazio:
o follower que ainda espera **rouba a leadership** (vê
`leader_active=false` e o próprio `PendingWrite` ainda na fila) e
commita o leftover — incluindo-se. Zero grupos extra no `put()` de
quem já teve Ok. Sem timed condvar (Darwin coalescing, RFC-0180 P0.15).

## Delivery slices (mandatory)

### P0 — local, hang fechado

- [x] **P0.1** Este RFC — status: `done`
- [ ] **P0.2** Teste que **pende** sem o steal: N writers, 1 put cada,
      barreira, o último chega depois do WAL do grupo. `join` com
      deadline. Canário 8×32 não chega. — status: `todo`
- [ ] **P0.3** Steal no resign: leftover não `recv()` para sempre;
      overwrite_mc4 avg_group fica ≥2,3 (não o 1,95 do P0.36) —
      status: `todo`

### P1 — recover / caixa

- [ ] **P1.1** `async_concurrent_writers_recover` + steal ainda
      entrega todas as keys — status: `todo`
- [ ] **P1.2** overwrite_mc4 isolado 1-run Darwin avg ≥2,3 e sem
      hang no shutdown do bench — status: `todo`

### P2 — polish

- [ ] **P2.1** none yet — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este ficheiro | 2026-09-07 |
| P0.2 | p0 | teste que pende sem steal | todo | — | 2026-09-07 |
| P0.3 | p0 | steal no resign | todo | — | 2026-09-07 |
| P1.1 | p1 | recover + keys | todo | — | 2026-09-07 |
| P1.2 | p1 | overwrite avg ≥2,3 | todo | — | 2026-09-07 |
| P2.1 | p2 | none yet | todo | — | 2026-09-07 |

## Acceptance Criteria

- **Tests:** P0.2 falha (timeout) no HEAD actual; P0.3 faz passar.
  Não reintroduzir drain-in-lead (P0.36).
- **Telemetry / Analytics:** `write_group` avg_group no log do
  overwrite_mc4; sem `PEDRA_STALL` `follower_recv` ≥1 s no shutdown.
- **Documentation:** este RFC; 0180 P0.36 aponta para cá.
- **Screenshots:** backend-only.

## Out of scope

- G1 como win. Peer `sync=true`. Fjall como gate.
- Concurrent skiplist. WAL on-lock (0180 P0.4/P0.28).
- Bake 4 GiB. RFC-0175.
