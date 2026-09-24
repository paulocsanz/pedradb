# RFC: O drain do WAL reusa o buffer em vez de alocar por put

**Status:** in-progress
**Updated:** 2026-09-23

## Background
- O acerto do mmap (RFC-0244) deixou a fase `wal` em 1,23 µs num put de 8000 operações (wall 2,90 µs). `mem` ficou em 0,36 µs.
- Sonda fora do motor (160 B, 200000 vezes, página de 16 KiB): cópia `MAP_SHARED` 0,02 µs, `pwrite` 1,3 µs, `write` 1,3 µs. A falta de página não é o dono.
- O mesmo put, com o relógio partido dentro de `write_pending_frame_lone`: `stage` 0,02 µs, `drain` 0,10–0,19 µs, e `reserve` 3,8–11,7 µs com **dois** `F_PREALLOCATE` de 64 MiB (`prealloc_ok=2`). O 1,23 µs do micro é essa reserva diluída em 8000 puts, não trabalho por put.
- O caminho lone drena o staging a cada frame. `drain_staged` fazia `mem::take` e, no sucesso, largava o `Vec`. O frame seguinte alocava de novo.

## Problems This Solves
- **Problem:** cada put de um escritor só paga um malloc e um free do buffer de staging, em cima de uma cópia que já cabe no mapa.

## Proposed Solution
- No sucesso, o drain devolve o `Vec` e dá `clear`, guardando a capacidade. No erro, devolve o `Vec` com os bytes, como antes. O sink continua recebendo os mesmos bytes antes do Ok.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** Drain lone reusa a capacidade do buffer de staging — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [ ] **P1.1** Re-meter C2, A4 e A5 na caixa com a imagem que inclui este corte e o RFC-0244 — status: `todo`

### P2 — later / polish
- [x] **P2.1** O índice de chave crua foi para o mapa de ponto com o scan ordenado (RFC-0246) — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | drain reusa o buffer | done | este change | 2026-09-23 |
| P1.1 | p1 | re-meter Linux | todo | — | 2026-09-23 |
| P2.1 | p2 | BTree do prefixo vazio virou o mapa de ponto | done | RFC-0246 | 2026-09-23 |

## Acceptance Criteria
- **Tests:** `rfc0245_repeated_lone_drain_keeps_staging_capacity` (três drains, a mesma capacidade, três writes, records iguais); `rfc0209_buffered_wal_byte_identical_after_drain` e `rfc0209_staging_coalesces_sink_writes` continuam.
- **Telemetry / Analytics:** none — a sonda de página foi um executável descartável, não um contador de produção.
- **Documentation:** este RFC. O P2.1 do RFC-0244 fica fechado: a falta de página mediu 0,02 µs.
- **Screenshots:** backend-only.

## Out of scope
- Não declara A4, A5 ou C2 pagos. Não há ratio Linux deste binário.
- Não desliga o staging no caminho de grupo (lá o buffer acumula até 64 KiB).
- Não trata o 1,23 µs do micro de 8000 como número de cartaz: medido, é dois `F_PREALLOCATE` diluídos. O trabalho por put no sink ficou em 0,10–0,19 µs.
