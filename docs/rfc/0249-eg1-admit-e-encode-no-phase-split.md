# RFC: Admit e encode entram no phase split

**Status:** in-progress
**Updated:** 2026-09-23

## Background
- `:p248` deixou A4 em mediana 0,2349. `:p249` (RFC-0248, família Dead sem realocar) na primeira rodada fez 65913 qps contra Rocks 232772 (0,283). A segunda rodada Pedra fez 87174. Não chega em 1,0.
- No mesmo guest, o A4 de 25M com os contadores antigos somava 6,45µs (mem 3,96, wal 1,64) e o qps pedia ~15–19µs de hold. O miolo sem cronómetro é admit + encode.
- A janela medida de `:p249` r2 quase não teve page fault (7 minor faults em ~10s, 1 núcleo). O tempo é CPU.

## Problems This Solves
- **Problem:** o phase split não nomeia o pedaço do commit que falta entre o prepare e o `write()` do WAL, então o corte seguinte seria outro palpite.

## Proposed Solution
- Com `PEDRA_WRITE_PHASE_STATS=1`, `admit_ns` cobre sonda de disco, park e observe do bulk; `encode_ns` cobre o vlog e o `encode_write_op_batches`. A onda pagante não liga o env. O corte do dono é o P1, depois de um A4 de 25M no guest.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** O bypass assíncrono de 1 op preenche `admit_ns` e `encode_ns` — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [ ] **P1.1** Um A4 de 25M no guest com o env ligado nomeia admit ou encode, e o corte desse dono entra no mesmo ciclo — status: `todo`

### P2 — later / polish
- [ ] **P2.1** Se mem continuar acima de 2µs depois do P1, o corte seguinte é o insert do mapa de ponto — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | admit e encode cronometrados | done | este change | 2026-09-23 |
| P1.1 | p1 | nomear e cortar o dono no guest | todo | — | 2026-09-23 |
| P2.1 | p2 | insert do mapa se mem sobrar | todo | — | 2026-09-23 |

## Acceptance Criteria
- **Tests:** `rfc0211_group_phase_stats_fill` — no braço bypass, `admit_ns > 0` e `encode_ns > 0` depois de puts reais.
- **Telemetry / Analytics:** os contadores só existem com `PEDRA_WRITE_PHASE_STATS=1`. A linha `phasesΔ` do multi-client imprime `admit` e `encode`.
- **Documentation:** este RFC. O percentual EG1 não muda: isto não fecha fatia.
- **Screenshots:** backend-only.

## Out of scope
- Ligar o env na onda de 3 rodadas.
- Tratar a subida de 65k–87k qps do `:p249` como A4 pago.
