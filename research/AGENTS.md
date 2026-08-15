# Convenções — `research/` (PedraDB)

Catálogo vivo de papers que devem **mudar ou recusar** decisões do
Pedra / Montanha / camadas. Nada aqui é revisão fechada.

Metodologia herdada (não reinventada):

| De | O quê |
|----|--------|
| [`../coop`](../../coop/) | Fichamento + sinopse + correlação; estrato a/b/c; fonte no disco **na hora**; D0–D4; TIMELINE append-only; “vai” ≠ throughput; skill de auditoria; Ctrl+F ≠ lido |
| [`../determinismo/pedradb-dst`](../../determinismo/pedradb-dst/) | **Ledger** como fonte única do que tentou / vale / falhou / está bloqueado / é falso positivo — na mesma sessão, nunca só no chat |
| `mine-and-verify` / regra global de pesquisa | Abstract e survey são hipótese; número antes de teoria; fonte primária persistida |

Norma de profundidade: [`QUALIDADE.md`](QUALIDADE.md).
Skill: [`../.grok/skills/audit-qualidade/SKILL.md`](../.grok/skills/audit-qualidade/SKILL.md).

## Regras

1. **Uma frente por sessão.** Um paper D3 (ou um lote de *fetch* sem fingir ficha) + TIMELINE + PLANO. Não “os 100”.
2. **Tese/número de um paper exige locus** (p. / Fig. / Table / §). Sem aspas+locus, é paráfrase — marcar.
3. **Três estratos, sempre:** (a) PDF integral; (b) parcial + residual; (c) secundário. Corpus no disco ≠ (a).
4. **Eixo deste catálogo** (seção *Relação com Pedra*): o paper informa um crate/RFC, um *measure*, ou um *refuse*? Se não, escrever “não se aplica” — não forçar compact/HTAP na manchete.
5. **Status de acesso visível** em `bibliografia.md` / `catalog.tsv` (✅📄⚠️❌).
6. **Não apagar erro.** Corrigir no sítio e deixar uma linha no TIMELINE / ficha (“correção YYYY-MM-DD”).
7. **“vai”** = próximo passo do `PLANO.md` em D3+. Intercalar `/audit-qualidade` se a sessão anterior fechou fichas ou há dívida &lt; D3.
8. **Ficha nova não é “fechada” sem** `score_fichamento.py --file …` ≥ D3.
9. **Keyword search no PDF ≠ lido.** Baixar → fichar → sinopse → correlacionar → *só então* “confirma”.
10. **Achado sem write-back não existe.** Fonte em `fontes/` ou `docs/references/`; decisão em `LEDGER.md`; sessão em `TIMELINE.md`. Scratch em `/tmp` ou só no chat = não aconteceu.

## Nomenclatura

| Peça | Nome |
|------|------|
| Fonte | `fontes/Rxxx_Autor_Ano_Slug.pdf` (+ `.txt` se extraído) |
| Já no repo | **não copiar** — apontar a `docs/references/…` |
| Ficha | `fichamentos/ficha_Rxxx_Autor_Slug.md` |
| Sinopse | `sinopses/sinopse_Rxxx_Autor_Slug.md` |
| Id | o mesmo `Rxxx` de `catalog.tsv` |

## O que não vive aqui

- Auditoria de *código* do engine → skill `/audit-pedradb`
- Hunt DST / bugs → `../determinismo/pedradb-dst/findings/LEDGER.md`
- RFCs de produto → `docs/rfc/` (a ficha *aponta*; não substitui o RFC)
