# Research — PedraDB / Montanha

**Norma:** [`QUALIDADE.md`](QUALIDADE.md) · **sessão:** [`AGENTS.md`](AGENTS.md)
**Próximo passo:** [`PLANO.md`](PLANO.md) · **decisões:** [`LEDGER.md`](LEDGER.md)

Catálogo vivo. Um paper **não está lido** até existir `fichamentos/ficha_Rxxx_*.md` ≥ **D3**. Abstract, survey e Ctrl+F são hipótese.

Metodologia herdada de [`../coop`](../../coop/) (fichamento / auditoria / TIMELINE) e do ledger DST em [`../determinismo/pedradb-dst`](../../determinismo/pedradb-dst/).

## Mapa

| Path | Papel |
|------|--------|
| [`QUALIDADE.md`](QUALIDADE.md) | D0–D4; “fechado” só D3+ |
| [`AGENTS.md`](AGENTS.md) | regras de sessão (vai, estrato, persistir) |
| [`CATALOG.md`](CATALOG.md) / [`catalog.tsv`](catalog.tsv) | os 100 |
| [`bibliografia.md`](bibliografia.md) | acesso ✅📄⚠️❌ |
| [`fontes/`](fontes/) | PDF/txt brutos (PDFs gitignored) |
| [`fichamentos/`](fichamentos/) | leitura com citas + locus |
| [`sinopses/`](sinopses/) | 1 página, só depois de D3 |
| [`correlacao/`](correlacao/) | cruza fichas; estratégias (ainda pré-D3) |
| [`LEDGER.md`](LEDGER.md) | SHIP / MEASURE / REFUSE / OPEN |
| [`PLANO.md`](PLANO.md) | o que o “vai” executa |
| [`TIMELINE.md`](TIMELINE.md) | append-only |
| [`BACKLOG.md`](BACKLOG.md) | candidatos 101+ |
| [`mapa/`](mapa/) | como a lista foi feita |
| skill [`/audit-qualidade`](../.grok/skills/audit-qualidade/SKILL.md) | score + elevação |

PDFs que já existem: [`docs/references/`](../docs/references/). Não duplicar.

## Camadas

```
3. Databases on top     SQL · HTAP · fold · DCS · stream
2. Distributed          Raft / FDB-roles · TX · DST
1. Local engine + DS    LSM · filters · WAL · vlog · compact
```

## Fluxo

```
fetch-one.sh  →  ficha template  →  score ≥ D3  →  sinopse
              →  catalog.tsv=ficha  →  LEDGER se decidiu  →  TIMELINE
```

```bash
python3 .grok/skills/audit-qualidade/scripts/score_fichamento.py --root research
./research/scripts/fetch-one.sh R010 https://www.usenix.org/system/files/fast21-dong.pdf Dong_2021_RocksExperience
```

## Já decidido (ledger; fichas ainda em dívida)

Bloom shipped · vlog spill shipped / GC open · Lazy Leveling recusado ·
BTreeMap memtable · coluna ≠ segundo primary · fold = LocalApplied.

Reabrir só com ficha D3 + número (`LEDGER.md`, RFC-0012).
