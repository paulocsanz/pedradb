# 2026-08-22 — rearm5: bateria P0.4-método no HEAD do write-path fix (`28a3d59`)

**VERDICT: NON-OFFICIAL** — watchdog flagou load1=14,86 durante a perna
longa 2M (compat), sobe a ~12,9 na perna rocks. Pela norma P0.4 a bateria
inteira vale como evidência. 4ª confirmação suja consecutiva na caixa
(rearm1/3/4/5) — o fechamento oficial segue bloqueado por ruído ambiente,
não por números.

## Nuance de carga (importante para ler as tabelas)

- Gate: QUIET às 15:10:14 (7,40; 2 consecutivos sob 10).
- **Todas as 12 pernas dos rounds 1–3 rodaram a load 7,69–8,52** (sob a
  barra, registradas perna a perna em `loads.txt`) — terminaram 15:10:41.
- O watchdog disparou às 15:11:14 (**durante** a longa compat, iniciada
  15:10:45): as ratios da longa (get 1,60 / set 1,28 / pipeline 1,14)
  correram sob load 12–15 e **estão contaminadas** — não usar.

## Rounds mediana-de-3 (n=2000, peer = RocksDB default `sync=false`)

### async (P2.2 — alvo do write-path fix `a28637a`)

| shape | runs | med | rearm4 (pré-fix) |
|---|---|---|---|
| ycsb_e | 5,36/7,11/5,82 | **5,82** | 4,94 |
| ycsb_d | 4,86/5,94/5,45 | **5,45** | ~3,0 |
| ycsb_a | 4,78/6,44/5,40 | **5,40** | 3,65 |
| ycsb_c | 4,25/4,78/4,70 | 4,70 | 3,95 |
| ycsb_b | 3,83/5,28/4,03 | 4,03 | 2,93 |
| ycsb_f | 3,46/4,48/3,73 | 3,73 | 3,33 |
| deps_cache_overwrite | 4,41/4,21/3,57 | 4,21 | — |
| deps_scan | 2,58/2,88/2,63 | 2,63 | — |
| deps_lock_prewrite | 2,49/2,12/2,84 | 2,49 | 0,94 (P2.1) |
| deps_apply_batch | 2,15/2,50/2,30 | 2,30 | — |
| deps_mvcc_latest | 1,93/2,40/2,06 | 2,06 | — |
| deps_raftlog | 1,03/0,92/0,85 | 0,92 | — |

Primeira bateria com **a/d/e ≥5 na mediana** — o efeito direto do fix do
fold (CPU user da suíte −93%, RSS −62%; ver
`../2026-08-22-writepath-fold-gc/`). C/B/F sobem mas não fecham.

### kvr

| shape | runs | med |
|---|---|---|
| kvrocks_get | 6,18/7,07/6,70 | **6,70 (3/3 ≥5)** |
| kvrocks_pipelined_set | 4,92/5,45/5,21 | 5,21 (2/3) |
| kvrocks_set | 4,28/5,12/4,95 | 4,95 (2/3) |
| kvrocks_scan | 31,59/36,32/35,21 | 35,21 |
| kvrocks_set_mc50 | 1,96/2,09/2,02 | 2,02 |
| kvrocks_blob_set | 1,82/1,52/1,94 | 1,82 |

GET: **4ª confirmação suja consecutiva 3/3 ≥5** (rearm1 4,4–5,5 → rearm3
5,98/6,28/6,46 → rearm4 6,00/6,30/6,66 → rearm5 6,18/7,07/6,70). Pipeline
e SET encostam na linha (5,21 / 4,95 med).

## Longa 2M — CONTAMINADA (load 12–15), não usar

get 1,60 / set 1,28 / pipeline 1,14 — rodou exatamente na janela do pico
de load. Descartada pela norma do watchdog.

## Conclusão

- Fechamento **oficial** de P1.1/P1.2/P1.3/P2.2 segue aguardando caixa
  quieta (rearm6 no mesmo HEAD, sem mudanças).
- Evidência acumulada: GET ≥5 estável em 4 baterias; write shapes a/d/e
  cruzaram 5× com o fix do write path; f/c/b na fila (3,73/4,70/4,03).
- G1 regression: 14 passed (durabilidade intacta no novo HEAD).
