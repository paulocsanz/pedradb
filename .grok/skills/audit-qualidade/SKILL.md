---
name: audit-qualidade
description: >
  Audit PedraDB research-catalog fichamentos against research/QUALIDADE.md
  depth tiers D0–D4 (meta: every ficha ≥ D3). Use when the user asks to
  audit quality, check fichamento, D3, qualidade, "fichar", "vai" on
  research/, or runs /audit-qualidade. Also before calling a paper ficha
  "fechada", and intermittently when the next research step is to audit
  recent fichas rather than open a new paper.
---

# /audit-qualidade — catálogo `research/` (D0–D4, meta ≥ D3)

Garante que `research/fichamentos/` cumpre [`research/QUALIDADE.md`](../../../research/QUALIDADE.md).
Espelho da skill em `../coop/.agents/skills/audit-qualidade`, **sem** os
eixos de livro/filologia que não se aplicam a papers.

Convenções de sessão: [`research/AGENTS.md`](../../../research/AGENTS.md).
Ledger de decisões: [`research/LEDGER.md`](../../../research/LEDGER.md).

| Tier | Significado |
|------|-------------|
| **D4** | D3 + lido o PDF original (marcador D4-b) |
| **D3** | **Meta mínima** — ficha válida |
| **D2** | Amostra (📄) |
| **D1** | Nota / ponte |
| **D0** | Inválido |

“Fechado” só D3/D4.

## When

- User: “audita qualidade”, “D3”, “fichar”, `/audit-qualidade`
- “vai” em research se a sessão anterior fechou fichas ou há dívida &lt; D3
- Antes de promover uma linha a `SHIP`/`REFUSE` no LEDGER com `src=ficha`

## Steps

### 1. Score mecânico

Da **raiz do pedradb**:

```bash
python3 .grok/skills/audit-qualidade/scripts/score_fichamento.py --root research
python3 .grok/skills/audit-qualidade/scripts/score_fichamento.py --only-below D3
python3 .grok/skills/audit-qualidade/scripts/score_fichamento.py --file research/fichamentos/ficha_R005_Lu_WiscKey.md
```

Necessário, **não** suficiente.

### 2. Conferir à mão (toda ficha abaixo de D3 ou recém-fechada)

- Fonte local existe (`fontes/` ou `docs/references/`)
- ≥1 citação amostrada **no PDF** (não inventar)
- Estrato (b) declara residual
- Relação com Pedra não força compact/HTAP
- Números têm Fig./Table/p.

### 3. Elevar se o user pediu “todos D3”

Ordem: D0 → D1 → D2. Relê o PDF. Re-score. Sinopse só depois de D3.
LEDGER na mesma mudança se a tese mudou.

### 4. Reportar

| Path | Antes | Depois | Notas |
|------|-------|--------|-------|
| … | listed | D3 210L | … |

Achados por faixa **MENOR / SIGNIFICATIVO / CRÍTICO** (`QUALIDADE.md` §8).
Zero CRÍTICO explícito. TIMELINE se elevou.

Não é esta skill: auditoria de *código* → `/audit-pedradb`.

## Hard rules

- Aspas só de fonte local (ou estrato c declarado)
- Ctrl+F / grep no PDF ≠ lido
- PDF no disco ≠ D3
- “Fechado” só D3/D4
- “vai” intercala esta skill (`AGENTS.md` regra 7)
- Não citar survey como se fosse o paper citado

## Gold

Estado (audit 2026-08-23): **17 fichas D4**, zero < D3 — meta ≥D3 atingida.
Padrão de *forma*: `research/fichamentos/template.md`. Padrão de *rigor
citacional* (outro domínio): `../coop/referencias/fichamentos/template.md`.
Nota de verificação: extrações `pages.txt` podem hifenizar, usar ligaduras
(ﬁ) ou intercalar colunas — grep literal falha; normalizar (NFKC, de-hifen,
marcadores de página) antes de concluir MISS.
