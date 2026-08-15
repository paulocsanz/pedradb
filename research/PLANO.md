# PLANO — o que fazer a seguir

Vivo. Atualizar no fim de cada sessão. “vai” = o topo desta lista, em D3+.
Não inventar uma fila paralela de “clássicos leves”.

Norma: [`QUALIDADE.md`](QUALIDADE.md) · fila longa: [`queue.md`](queue.md)
(histórico das waves) · decisões: [`LEDGER.md`](LEDGER.md).

## Agora (Wave 1)

Wave 0 fechada. Lethe (R031) fechado. Próximo: Disco/REMIX (L13)
se `scan` doer; senão o próximo listed da Wave 1 (R023 Disco).

| Pri | ID | Ação | Por quê |
|----:|----|------|---------|
| 1 | R023 | fetch + fichar Disco | compact multi-run index; L13 |

Depois de cada ficha: `score_fichamento.py --file …`, sinopse, `catalog.tsv
status=ficha`, linha no LEDGER se a decisão mudou, TIMELINE.

## Depois (Wave 1–2)

Topo após Disco: REMIX (R022) se o scan doer de verdade. SQL-on-KV
(F1 R058) não é P0.

## Dívida de qualidade

- ~76 `listed` / **16 fichas D4**
- `correlacao/00_estrategias.md` continua pré-ficha para o resto

### Fechado nesta frente

| ID | Tier | Ledger |
|----|------|--------|
| R005 | D4 | L4 confirmado; L5 cisão (rewrite SHIP / incremental OPEN); L5b REFUSE drop WAL |
| R006 | D4 | L1 confirmado (Bloom-por-SST); L2 FPR Monkey **continua MEASURE** (HDD / \(L\le 4\)) |
| R007 | D4 | L3 Lazy Leveling **REFUSE** confirmado (exige L2; short range; \(L\le 4\)) |
| R041 | D4 | L22 protocolo Percolator `REFUSE`; L23 oracle-serviço `REFUSE`; OCC kernel > SI |
| R048 | D4 | L8 fold≠voter confirmado; L24 read-index no fold `REFUSE`; TiFlash ≠ fold |
| R010 | D4 | L9 confirmado; espaço>WA; L25–L27 MEASURE |
| R043 | D4 | L9 layers; L28 swarm MEASURE; L29 role-split REFUSE; L30 janela 5 s REFUSE |
| R044 | D4 | L9 layers; L31 same-TX idx SHIP; L32 produto RL REFUSE; L33 VERSION MEASURE; L34 atomics REFUSE |
| R012 | D4 | L35 LO+1 MEASURE; L36 menu REFUSE; L37 universal REFUSE; L38 TSD MEASURE |
| R045 | D4 | L9/L29 confirmados; L39 lease MEASURE; L40 HLC/intents CRDB REFUSE |
| R018 | D4 | L19 hash-groups MEASURE (0028 não P0); L4/L5b confirmados; tail = Fig. 2 |
| R013 | D4 | L11 Spooky MEASURE depois de L35; Full-do-par ≠ Full-de-\(L\) |
| R014 | D4 | L12 robust static MEASURE (sem knobs hoje); Table 3 confirma L3/L37 |
| R017 | D4 | L41 stall names + flush>L0 MEASURE; não SILK completo |
| R016 | D4 | L41 taxonomia MMO/L0O/RDO; L42 tuner REFUSE |
| R031 | D4 | L38 FADE MEASURE (depois L35, com SLA); L43 KiWi REFUSE |

## Fora de linha (à espera do Paulo)

RFC-0026/0029 P0 done. **16 fichas D4**. Próximo: **R023 Disco**.
L38/L43: não implementar FADE nem KiWi agora.

## Fora de escopo deste plano

- Implementar compact/vlog porque um paper é famoso
- Auditar código (`/audit-pedradb`)
- Baixar os 100 PDFs de uma vez
