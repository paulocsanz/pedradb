# RFC-0222 — baseline sel4_gap @ eec62693 (2026-09-13)

Métrica de máquina: `python3 scripts/sel4_gap.py` (denominador nomeado por eixo).
Baseline completo em `baseline-20260913.txt`. Números-chave:

| Eixo | Baseline | Destino datado |
|---|---|---|
| A1 refinamento topo | 0 | sem data — pesquisa (via RFC-0220) |
| A2a superfície kernel LOC | 27.960/164.359 = 17,01% | sobe com o dreno de glue (0219/0220) |
| A2b fns de kernel na superfície | 801/857 = 93,47% (61/68 arquivos) | 100% — 2026-10-15 (onda de enrollment P0.7) |
| A3 confinamento | 0 | sem data — pesquisa |
| A4 espinha de recovery | 0/11 encadeados | ≥1 espinha — 2026-10-31 (P2.2) |
| A6 ∀-concorrência | 0 | sem data — pesquisa |
| A8 classe de claim | 1 (cânone) | mantém 1 |
| A9a gates baratos | 6/8 (barrier+clock vermelhos) | 8/8 — P0.2/P0.3 |
| A9b CI GitHub | 0 | 1 — P0.8 (push é portão do usuário) |
| A10 stdlib sorries no TCB | 0 | 1 — P0.4 |
| bloco DEFINING | 18,41% | teto de engenharia ~60–70%; resto pesquisa |
| bloco CLAIM+EVIDÊNCIA | 43,75% | 100% no Piso Verde (P0) |

`m2` composição 33/286 = 11,54% bate com o baseline medido do RFC-0220 (commit f17d3621) — a métrica nova re-deriva o mesmo número por caminho independente (TSV + grep Compose* vs contagem manual do 0220).
