# RFC: O acerto do WAL mmap não pega o mutex global

**Status:** in-progress
**Updated:** 2026-09-22

## Background
- C2 no `:p244` ficou em 241788 qps contra fjall 396321 (~4,1 µs contra ~2,5 µs).
- Release local do put de um escritor, fase a fase: `wal` 1,87 µs de um wall de 3,00 µs. `mem` ficou em 0,48 µs. 7999 de 8000 frames acertaram um mapa já criado e mesmo assim tomaram o mutex global do mapa.
- A caixa em Brasil está sem capacidade, então o re-meter do RFC-0243 não rodou.

## Problems This Solves
- **Problem:** cada frame pequeno do WAL paga o lock e o HashMap do mapa compartilhado mesmo quando o handle já tem o ponteiro.

## Proposed Solution
- O handle guarda o endereço e o tamanho do mapa. O frame que cabe copia sem o mutex. Crescer o mapa continua no caminho lento e atualiza o handle.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** Acerto do mmap copia pelo ponteiro do handle — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [ ] **P1.1** Re-meter C2, A4 e A5 na caixa quando Brasil tiver capacidade, imagem com RFC-0243 e este corte — status: `todo`

### P2 — later / polish
- [x] **P2.1** A falta de página do `MAP_SHARED` foi medida e não é o dono (cópia 0,02 µs, `pwrite` 1,3 µs). O dono seguinte é o malloc por put do staging (RFC-0245) — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | cópia sem o mutex global | done | este change | 2026-09-22 |
| P1.1 | p1 | re-meter Linux | todo | — | 2026-09-22 |
| P2.1 | p2 | falta de página medida, não é o dono | done | RFC-0245 | 2026-09-23 |

## Acceptance Criteria
- **Tests:** `rfc0244_phase_split_records_every_put` (quase todo frame no caminho rápido, put legível); `rfc0242_small_puts_skip_per_op_pwrite_and_reopen` (reopen e stat ≤ 8).
- **Telemetry / Analytics:** none — contadores só para o teste.
- **Documentation:** este RFC.
- **Screenshots:** backend-only.

## Out of scope
- Não declara C2 pago. O número Linux ainda não existe para este binário.
- Não muda o contrato de page cache antes do Ok.
