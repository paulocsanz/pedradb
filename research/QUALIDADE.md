# Qualidade do catálogo de papers — D0–D3 (D4 = PDF original)

**Obrigatório** para quem produza ou edite `research/fichamentos/`,
`research/sinopses/`, `research/correlacao/`.

**Origem:** espelho de [`../coop/QUALIDADE.md`](../../coop/QUALIDADE.md)
(D0–D4, skill `/audit-qualidade`). Enxugado para *papers* de sistemas
(SIGMOD/VLDB/FAST/OSDI…), não livros de 400 p.

**Auditoria:** skill **`/audit-qualidade`**
([`.grok/skills/audit-qualidade/`](../.grok/skills/audit-qualidade/)).

**“Fechado” só com D3 (ou D4).** Abstract, survey paragraph, HTML landing
page, ou Ctrl+F no PDF **não** são fichamento.

---

## 1. Hierarquia

| Nível | Nome | Exigência mínima |
|-------|------|------------------|
| **D4** | D3 + original | Tudo de D3 **e** declaração explícita: lido o **PDF/HTML da obra**, na língua em que foi escrita (quase sempre EN). Sem essa frase, fica D3. |
| **D3** | Fichamento válido | Template completo; estrato a/b/c; fonte local; ≥**4** citas literais com locus (fig/table/§/p.); resumo seção a seção; “o que **não** foi lido” se (b); **1 paper por ficheiro** |
| **D2** | Amostra honesta | Fonte local; citas com locus; escopo no título e no topo (📄); não fingir monografia |
| **D1** | Nota / ponte | Link em `fontes/` ou `docs/references/`; o que se tentou; **sem** teses longas; prefixo `nota_` |
| **D0** | Inválido | Sem citas; abstract como fato; multi-paper fingido; estrato mentiroso |

**Meta:** todo `ficha_R*.md` ≥ **D3**. D2 só com banner e plano de elevação.
D1/D0 não são “quase bom”.

Papers CS são curtos. O teto filológico (D4-a cotejo de tradução) quase
nunca se aplica. **D4-b** = leu o PDF, não o arXiv HTML de 3 parágrafos.

---

## 2. Unidades

| Unidade | Onde | Padrão |
|---------|------|--------|
| **Fonte bruta** | `fontes/` (PDF/txt) ou `docs/references/` | No disco **antes** de “fechado” |
| **Fichamento** | `fichamentos/ficha_Rxxx_*.md` | **D3** mínimo |
| **Sinopse** | `sinopses/` | Condensado de ficha **já D3**; não substitui |
| **Correlação** | `correlacao/` | Cruza fichas D3; hipótese vs. verificado |
| **Ledger** | `LEDGER.md` | Ship / measure / refuse / blocked — só depois da ficha, ou como hipótese marcada |
| **Bibliografia** | `bibliografia.md` + `catalog.tsv` | status ✅📄⚠️❌ |
| **TIMELINE** | `TIMELINE.md` | append-only por sessão |
| **PLANO** | `PLANO.md` | próximo passo; não inventar fila paralela |

---

## 3. Checklist D3 (paper)

Todas verdade:

1. **Fonte local** existente (`fontes/Rxxx_…pdf` **ou** `docs/references/…`).
2. **Template** (`fichamentos/template.md`) com seções reconhecíveis:
   Referência · Dados da leitura (**Estrato**) · Resumo · Tese · Estrutura ·
   Conceitos · **Citações relevantes** · Diálogo · Relação com Pedra ·
   Avaliação · Palavras-chave · Fontes.
3. **≥ 4** citas literais numeradas `1. "…" (p. N / Fig. X / §Y)` —
   preferir ≥8 em papers longos (≥16 p.).
4. **Extensão orientativa** (anti-stub, sem padding):
   - paper de conferência: **≥ 180** linhas no `.md`
   - survey / TOCS / journal longo: **≥ 250**
5. **Estrato honesto:** (a) PDF integral; (b) parcial + o que não foi lido;
   (c) só secundário (survey que cita o paper). “PDF no disco” ≠ lido.
6. **Um paper por ficheiro.** O `id` (`Rxxx`) é o mesmo em fontes / ficha /
   sinopse / `catalog.tsv`.
7. Números da ficha têm **página/figura**. Throughput de abstract = D0.

Falha → não é D3.

### D4 (opcional)

Em “Dados da leitura”: *lido o PDF original em inglês (sem tradução
interposta)*. Marcador auditável:

```markdown
**Tier: D4** — via **D4-b** (PDF original EN)
```

---

## 4. Status (bibliografia / topo da ficha)

| Símbolo | Significado |
|---------|-------------|
| ✅ | **D3** (ou D4) — sinopse permitida |
| 📄 | **D2** — amostra; citar só trechos com locus |
| ⚠️ | **D1** — ponte; não prova de leitura |
| ❌ | Sem acesso / **D0** / só listado no catálogo |

`catalog.tsv` `status=listed` = ❌ até haver ficha.

---

## 5. O que **não** conta

- Catálogo de 100 títulos sem ficha.
- Ctrl+F / grep no PDF (“o paper menciona compaction”).
- Landing page USENIX / arXiv HTML de abstract.
- Survey (Lv 2025, Zhang 2024) como prova do paper que ele cita.
- “Fecho operacional” / `máx. NNN` sem D3.
- Sinopse a passar por fichamento.
- Forçar “isso muda o Pedra” na manchete — a seção Relação pode ser
  **não se aplica / recusar**.
- Throughput sob **“vai”**.

---

## 6. Política de “vai”

| Comando | Significado |
|---------|-------------|
| **vai** / continua / segue | Próximo passo do `PLANO.md` **no rigor D3+** |
| **não** | Throughput; stubs; “fechado” sem D3 |

Procedimento: ler `TIMELINE.md` (fim) + `PLANO.md`. Intercalar
`/audit-qualidade` se a sessão anterior fechou fichas em lote ou há
dívida &lt; D3. Dívida **precede** paper novo.

```bash
python3 .grok/skills/audit-qualidade/scripts/score_fichamento.py --root research
python3 .grok/skills/audit-qualidade/scripts/score_fichamento.py --only-below D3
python3 .grok/skills/audit-qualidade/scripts/score_fichamento.py --file research/fichamentos/ficha_R005_….md
```

---

## 7. Fluxo por paper

```
1. Fonte em fontes/ ou docs/references/   (fetch-one.sh)
2. Escopo da sessão (integral / §§ nomeados)
3. Citações literais + locus
4. Fichamento template → score ≥ D3
5. Sinopse
6. catalog.tsv status=ficha · bibliografia ✅
7. Se a leitura fecha uma decisão: LEDGER.md na mesma mudança
8. correlacao/ se cruza ≥2 fichas D3
9. TIMELINE.md append
```

**Antes de citar um paper como “confirma X”:** baixar → fichar D3 →
sinopse → correlacionar. Qualquer etapa em falta = ❌ na bibliografia,
nunca “achado real”.

---

## 8. Severidade de achados (auditoria)

Não reportar “N problemas” sem faixa:

| Faixa | Significado |
|-------|-------------|
| **MENOR** | página/figura errada, citação real; OCR sem `[sic]` |
| **SIGNIFICATIVO** | nuance/hedge do paper perdido; número sem locus; tese inflada |
| **CRÍTICO** | citação fabricada; inverte a tese; “confirma” sem ter lido |

Score mecânico é **necessário, não suficiente**. Zero CRÍTICO deve
aparecer explícito no relatório.

---

## 9. Banners

**D0 / D1:**

```markdown
> ⚠️ **STATUS: NÃO É FICHAMENTO VÁLIDO (D0/D1).**
> Não citar como prova de leitura. Ver `research/QUALIDADE.md`.
```

**D2:**

```markdown
> 📄 **STATUS: AMOSTRA SELETIVA (D2) — não leitura integral.**
> Citar só trechos com locus. Meta: elevar a D3.
```
