# RFC: O bypass lone grava o frame uma vez

**Status:** in-progress
**Updated:** 2026-09-23

## Background
- `:p252` (RFC-0251) deixou A4 em mediana válida 0,7175 e A5 em 0,9584. C2 ficou com o fjall colapsado. EG1 segue 62%.
- Um A4 de 25M na mesma imagem, com phase stats ligados (não é cartaz), deu qps 122737. As fases por commit: prepare 0,08, wal 1,24, mem 0,75, publish 0,22, flush 0,13, admit 0,34, encode 0,33 µs. `lock_wait` 22,84 µs é dessa corrida com os contadores ligados.
- O maior pedaço nomeado dentro do write lock é o WAL, 1,24 µs. O bypass lone chama `write_frame` (copia para o buffer de staging) e em seguida `drain_staged` (copia de novo para o sink). O grupo precisa do staging. O lone drena todo frame.

## Problems This Solves
- **Problem:** cada put do bypass paga duas cópias do mesmo frame WAL.

## Proposed Solution
- `write_frame(..., sink_now)` drena o que já estava staged e grava este frame direto no sink.
- O caminho lone passa `sink_now`. O caminho de grupo continua no staging até o cap.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** O frame lone chega ao sink numa escrita, sem ficar no buffer de staging — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [x] **P1.1** Onda Linux de 3 rodadas, sem phase stats, A4/A5/C2, canário ≥165000, peer `sync:false` — status: `done`

### P2 — later / polish
- [x] **P2.1** O A4 de 25M ficou abaixo e um A4 curto no mesmo binário deu 257785 qps. O próximo corte é a janela de 64 MiB da mmap (RFC-0253) — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | lone grava o frame uma vez | done | RFC-0252 | 2026-09-23 |
| P1.1 | p1 | onda Linux A4/A5/C2 | done | :p253 | 2026-09-23 |
| P2.1 | p2 | dono que sobrar | done | RFC-0253 | 2026-09-23 |

## Acceptance Criteria
- **Tests:** `rfc0252_sink_now_skips_the_stage_copy` — um frame `sink_now` drena o stage anterior, grava o frame, deixa o buffer vazio, e um drain seguinte não escreve de novo. Com o stage vazio, o frame seguinte é uma escrita só. `rfc0209_buffered_wal_byte_identical_after_drain` e `rfc0245_repeated_lone_drain_keeps_staging_capacity` continuam válidos para o caminho que ainda drena.
- **Telemetry / Analytics:** nenhuma na onda pagante.
- **Documentation:** este RFC. O percentual EG1 não muda até a onda pagar uma fatia.
- **Screenshots:** backend-only.

## Out of scope
- Soltar o write lock do Db durante a cópia. Uma cópia fora de ordem abre um buraco no WAL: a recuperação para no CRC do offset anterior e perde um registro já confirmado.
- `PEDRA_WRITE_SPIN` e `PEDRA_WRITE_FAIR` como default.
