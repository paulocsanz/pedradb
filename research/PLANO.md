# PLANO — o que fazer a seguir

Vivo. Atualizar no fim de cada sessão. “vai” = o topo desta lista, em D3+.
Não inventar uma fila paralela de “clássicos leves”.

Norma: [`QUALIDADE.md`](QUALIDADE.md) · fila longa: [`queue.md`](queue.md)
(histórico das waves) · decisões: [`LEDGER.md`](LEDGER.md).

## Agora (Wave 0)

Fechar o loop das decisões que o repo **já cita**. Preferir fichar PDF
que já está em `docs/references/` (zero fetch) quando a sessão for curta.

| Pri | ID | Ação | Por quê |
|----:|----|------|---------|
| 1 | R006 | **fichar** Monkey (já local) | L1/L2 — Bloom shipped vs FPR |
| 3 | R007 | **fichar** Dostoevsky (já local) | L3 — REFUSE precisa de locus |
| 4 | R041 | **fichar** Percolator (já local) | 2PC/OCC do Montanha |
| 5 | R048 | **fichar** TiDB (já local) | learner ≠ voter / fold |
| 6 | R010 | fetch + fichar Rocks Experience | incumbent |
| 7 | R043 | fetch + fichar FoundationDB | Montanha + DST |
| 8 | R044 | fetch + fichar Record Layer | databases on KV |
| 9 | R012 | fetch + fichar compaction design space | 4 knobs |
| 10 | R045 | fetch + fichar CockroachDB 2020 | SQL + Multi-Raft |

Depois de cada ficha: `score_fichamento.py --file …`, sinopse, `catalog.tsv
status=ficha`, linha no LEDGER se a decisão mudou, TIMELINE.

## Depois (Wave 1–2)

Só com ≥5 fichas D3 na Wave 0. Ver `queue.md`.

## Dívida de qualidade

- 87 `listed` / 12 `have-pdf` / **1 ficha D4** (R005 WiscKey)
- `correlacao/00_estrategias.md` continua pré-ficha para o resto

### Fechado nesta frente

| ID | Tier | Ledger |
|----|------|--------|
| R005 | D4 | L4 confirmado; L5 cisão (rewrite SHIP / incremental OPEN); L5b REFUSE drop WAL |

## Fora de escopo deste plano

- Implementar compact/vlog porque um paper é famoso
- Auditar código (`/audit-pedradb`)
- Baixar os 100 PDFs de uma vez
