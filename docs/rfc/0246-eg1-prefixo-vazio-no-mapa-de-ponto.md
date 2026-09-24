# RFC: Chave crua entra no mapa de ponto, o scan continua ordenado

**Status:** in-progress
**Updated:** 2026-09-23

## Background
- A4 no `:p244` ficou em 0,1775 (38972 / 219603). A semente de 25M, um escritor, fez 85400 puts/s. C2 ficou em 241788 contra fjall 396321.
- O relógio partido do RFC-0245 mostrou que o sink do WAL, por put, é 0,10–0,19 µs. O que cresce com o número de chaves é o índice da chave crua: prefixo vazio, `BTreeMap` (`short`) em todo put.
- `lock` e `default` já usam o mapa Fx. O get de ponto olha esse mapa antes da árvore. O scan ordenado de um shard de ponto monta a vista uma vez (`point_ord_btree`).

## Problems This Solves
- **Problem:** cada put de chave crua (o shape do bench, sem `cf\0`) insere numa árvore cujo custo sobe com 1M e 25M chaves.

## Proposed Solution
- O prefixo vazio passa a ser shard de ponto, com reserva inicial. O get continua no mapa. O cursor de scan copia a faixa ordenada uma vez, a partir da vista preguiçosa, e não segura o lock dessa vista. `write` continua na árvore (o último visível é um reverse-seek).

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** Put de chave crua indexa o mapa de ponto e o scan dessa faixa sai ordenado — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [x] **P1.1** Re-meter A4, A5 e C2 na caixa com a imagem que inclui este corte — status: `done`
- `:p247`, 3/3 canário ≥165000, peer `sync:false`. A4 mediana 0,2276, A5 mediana 0,5666, C2 mediana pedra 296168 < fjall 321039. Nenhuma célula fecha. As chaves medidas são `ycsb/`, `c/` e `f/`. Seguimento: RFC-0247.

### P2 — later / polish
- [ ] **P2.1** Se o `ycsb_c` Linux cair (a tentativa antiga de HashMap no prefixo vazio perdeu 3× antes do hasher Fx e antes da vista ordenada separada do get), reverter só o prefixo vazio — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | chave crua no mapa, scan ordenado | done | este change | 2026-09-23 |
| P1.1 | p1 | re-meter Linux | done | `:p247` impago | 2026-09-23 |
| P2.1 | p2 | reverter se ycsb_c cair | todo | — | 2026-09-23 |

## Acceptance Criteria
- **Tests:** `rfc0246_raw_put_uses_point_map_and_scan_stays_sorted` (três puts fora de ordem, mapa com 3, árvore vazia, scan em ordem, overwrite não cresce o mapa); `insert_many_empty_prefix_count_matches_gets` (contagem ordenada ainda bate com o get).
- **Telemetry / Analytics:** none.
- **Documentation:** este RFC.
- **Screenshots:** backend-only.

## Out of scope
- Não declara A4, A5 ou C2 pagos.
- Não move o shard `write` para o mapa.
- Não trata um ratio Darwin como cartaz.
