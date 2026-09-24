# RFC: Prefixo de uma barra entra no mapa de ponto

**Status:** in-progress
**Updated:** 2026-09-23

## Background
- O RFC-0246 moveu o prefixo vazio para o mapa de ponto. As chaves do bench não são vazias.
- `idx_prefix` de uma chave com exatamente uma `/` é o pedaço até a barra, inclusive: `ycsb/000001` → `ycsb/`, `c/000001` → `c/`, `f/0000001` → `f/`.
- A semente de A4/A5 escreve `ycsb/{i}` (25M num único `BTreeMap`). O overwrite medido escreve `c/{u}`. O C2 escreve `f/{i}` (1M). Os três shards continuavam na árvore.
- `write` (bytes antes do NUL) continua na árvore: o último visível é um reverse-seek.

## Problems This Solves
- **Problem:** o corte do índice que deveria baratear A4 e C2 não alcança as chaves que esses shapes realmente gravam.

## Proposed Solution
- Um prefixo de índice com exatamente uma barra, terminando nessa barra, passa a ser shard de ponto, com a mesma reserva do prefixo vazio. O get usa o mapa. O scan copia a vista ordenada uma vez. `write` não muda.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** `c/`, `ycsb/` e `f/` indexam o mapa de ponto; o scan de `c/` sai ordenado; overwrite não cresce o mapa; `write` fica na árvore — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [x] **P1.1** Re-meter A4, A5 e C2 numa imagem que inclua este corte, com canário ≥165000 e peer `sync:false` — status: `done`

### P2 — later / polish
- [ ] **P2.1** Se o `ycsb_c` Linux cair, reverter só `one_slash_point_prefix` — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | uma barra no mapa, scan ordenado | done | este change | 2026-09-23 |
| P1.1 | p1 | re-meter Linux | done | :p248 impago | 2026-09-23 |
| P2.1 | p2 | reverter se ycsb_c cair | todo | — | 2026-09-23 |

## Acceptance Criteria
- **Tests:** `rfc0247_one_slash_bench_keys_use_point_map` (puts fora de ordem em `c/`, shards `ycsb/` e `f/` no mapa, `write` na árvore, scan ordenado, overwrite não cresce).
- **Telemetry / Analytics:** none.
- **Documentation:** este RFC e a nota na escada EG1. O percentual não muda: não há meter Linux deste binário.
- **Screenshots:** backend-only.

## Out of scope
- Medir esta imagem por cima da onda `:p247`, que ainda está no binário do RFC-0246.
- Mover o CF `write`.
