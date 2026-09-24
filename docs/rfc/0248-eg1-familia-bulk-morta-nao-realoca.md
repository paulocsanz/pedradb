# RFC: Família bulk morta não realoca no put

**Status:** in-progress
**Updated:** 2026-09-23

## Background
- `:p248` (RFC-0247) não pagou A4/A5/C2. Canário 3/3 ≥165000, peer `sync:false`. A4 mediana **0,2349** (0,2093 / 0,2349 / 0,3195). A5 mediana **0,6582**. C2 pedra mediana 288064; o fjall das três rodadas colapsou (133k–185k) e não conta.
- No mesmo guest, um A4 de 1M com `PEDRA_WRITE_PHASE_STATS=1` mediu prepare 0,14µs + wal 0,41µs + mem 1,79µs + publish 0,39µs + flush 0,14µs e `lock_wait` 11,10µs. O qps foi 80297, o mesmo patamar do A4 de 25M (48k–80k). O trabalho cronometrado não explica a espera.
- A semente ascendente trava a família `default`. O overwrite `c/` desce e mata o latch. `judge` devolve Ineligible para Dead, e o braço Ineligible reescrevia Dead como Probing. `ratchet` fazia `family.to_owned()` em todo put, debaixo do write lock.

## Problems This Solves
- **Problem:** depois do primeiro overwrite abaixo do high-water, cada put do A4 aloca a chave da família e rearma um latch que o contrato diz ser permanente.

## Proposed Solution
- Família Dead devolve Ladder na entrada de `classify_family`, sem ratchet e sem reescrever o estado. Um put acima do high-water depois do kill continua na escada.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** Dead fica Dead; o put seguinte acima do high-water não volta a Bulk e não consulta o máximo do disco — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [x] **P1.1** Re-meter A4/A5/C2 no guest 4 vCPU com canário ≥165000 e peer `sync:false` — status: `done`

### P2 — later / polish
- [ ] **P2.1** Se o A4 continuar com `lock_wait` ≫ fases, o próximo corte é o que o phase split de 25M nomear (não outro mapa de prefixo) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Dead não realoca nem re-trava | done | este change | 2026-09-23 |
| P1.1 | p1 | re-meter Linux | done | :p249 impago | 2026-09-23 |
| P2.1 | p2 | próximo corte se o wait sobrar | todo | — | 2026-09-23 |

## Acceptance Criteria
- **Tests:** `descent_across_batches_kills_family` — o segundo put acima do high-water continua Ladder, a família não fica latched, o closure do máximo em disco não corre.
- **Telemetry / Analytics:** none — o phase split já existente (`PEDRA_WRITE_PHASE_STATS`) é o metro; não entra na onda pagante.
- **Documentation:** este RFC e a escada EG1. O percentual não muda até o P1.1 pagar uma fatia.
- **Screenshots:** backend-only.

## Out of scope
- Reativar o merge de grupo no `overwrite_mc4` (p244, `avg_group` 2,04, foi mais lento que o bypass).
- Tratar vitória contra Rocks `sync:true`, ou o fjall colapsado do `:p248` como C2 pago.
