# RFC: Submit no claro não toma o read lock do Db

**Status:** in-progress
**Updated:** 2026-09-23

## Background
- No A4 com worker de flush e write buffer de 256 MiB, a janela cronometrada não estaciona tabela e o stall de L0 está desligado. Mesmo assim cada put chama `assist_flush_debt`, `await_flush_debt` e `await_l0_park`.
- Esses três caminhos lêem o cap, os bytes estacionados e o limite de stall com `db.read()`. O writer segura o write lock durante o commit. A espera desses read locks não entra em `lock_wait`.
- O diagnóstico `:p250` (musl malloc) deu 80191 qps, soma das fases 4,88µs, `1/qps` 12,5µs. Com mimalloc (`:p251`, uma corrida, phase stats) subiu para 123044 qps: fases somam 3,33µs e `1/qps` é 8,13µs. Admit 0,36µs e encode 0,36µs não fecham o buraco.
- A rodada 1 da onda `:p251` (ainda em curso, não é mediana) deu A4 133359 / 214240 = 0,622, sync false, canário 230431. A5 r1 237685 / 299183 = 0,794. C2 r1 pedra 330398 contra fjall 153568, par colapsado. Nenhum desses números fecha célula.

## Problems This Solves
- **Problem:** com a dívida abaixo do cap e o stall desligado, cada put ainda serializa em dois a cinco `db.read()` atrás do writer.

## Proposed Solution
- Publicar cap, bytes estacionados e limite de stall em atômicos atualizados dentro do `&mut Db`, antes de soltar o write lock.
- `submit_one`, `submit_inner` e `assist_flush_debt` só entram nos awaits com lock quando o cap não foi publicado, o stall está armado, ou a dívida publicada está no cap.
- Cap `0` significa não publicado. `usize::MAX` significa sem cap. Stall `0` significa desligado.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** O caminho claro pula os read locks, e a dívida real ou o stall armado continuam no caminho com lock — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [x] **P1.1** Onda Linux de 3 rodadas, sem phase stats, A4/A5/C2, canário ≥165000, peer `sync:false` — status: `done`

### P2 — later / polish
- [ ] **P2.1** O A4 válido ficou em 0,7175. O próximo corte é o que o phase split desta imagem mostrar como o maior pedaço dentro do write lock — status: `doing`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | submit claro sem read lock | done | RFC-0251 | 2026-09-23 |
| P1.1 | p1 | onda Linux A4/A5/C2 | done | :p252 | 2026-09-23 |
| P2.1 | p2 | dono que sobrar depois do portão | doing | phase :p252 | 2026-09-23 |

## Acceptance Criteria
- **Tests:** `rfc0251_submit_gates_track_park_cap_and_stall` confere cap publicado, bytes após park e pop, stall armado e desligado, e um put no caminho claro. `submit_flush_debt_assists_with_worker_attached`, `submit_flush_debt_releases_on_materialize` e `rfc0167_l0_stall_parks_until_worker_drains` continuam parando quando a dívida ou o stall são reais. `parked_debt_plan_on_live_at_cap_parks` e `flusher_gate_plan_on_live_workerless_parks_nowhere` continuam achando o `match` nos trampolins.
- **Telemetry / Analytics:** nenhuma na onda pagante. `PEDRA_WRITE_PHASE_STATS` fica desligado.
- **Documentation:** este RFC. O percentual EG1 não muda neste corte: a onda `:p251` ainda não é mediana de 3 rodadas, e este binário ainda não rodou no guest.
- **Screenshots:** backend-only.

## Out of scope
- Ligar `PEDRA_WRITE_SPIN`, `PEDRA_ASYNC_GROUP` ou `PEDRA_WRITE_FAIR`. Os três pioraram o A4 neste guest.
- Publicar a contagem de arquivos L0. Com o stall desligado o await não precisa dela.
- Tratar a rodada 1 do `:p251` como cartaz.
