# TIMELINE — research/

Registro **append-only**. Uma entrada por sessão que tocou o catálogo.
Não reescrever o passado; corrigir em entrada nova.

---

## 2026-08-14 — pasta + 100 + metodologia coop

- Criado `research/` com catálogo ranqueado de 100 papers (engine /
  dist / layer / HTAP / TX / test), `catalog.tsv`, backlog, queue.
- 13 PDFs já em `docs/references/` marcados `have-pdf`. Zero fichas.
- Importada a metodologia de `../coop` (D0–D3, fonte no disco, TIMELINE,
  skill + score, Ctrl+F ≠ lido, “vai” ≠ throughput) e o **ledger** de
  `../determinismo/pedradb-dst` (tentou / vale / recusa / bloqueado).
- Estrutura: `QUALIDADE.md`, `AGENTS.md`, `fichamentos/`, `sinopses/`,
  `fontes/`, `correlacao/`, `LEDGER.md`, `PLANO.md`.
- Skill `.grok/skills/audit-qualidade` + `score_fichamento.py`.
- **Não feito:** nenhuma ficha D3; nenhum PDF novo baixado.

**Pendente (vai):** Wave 0 de `PLANO.md` — próximo: R006 Monkey.

---

## 2026-08-14 — ficha R005 WiscKey (D4)

- Lido `docs/references/wisckey-fast2016.pdf` na íntegra (§§1–6).
- `fichamentos/ficha_R005_Lu_WiscKey.md` — score `D4 328L citas=17`.
- Sinopse escrita. `catalog.tsv` / CATALOG / bibliografia → `ficha` / ✅.
- Ledger: L4 `src=ficha`; L5a rewrite `SHIP` (já era RFC-0016); L5
  incremental continua `OPEN`; **L5b `REFUSE`** dropar WAL da LSM
  (vLog Pedra não carrega a key no record — §3.4.2 não se aplica).
- RFC-0014 ainda diz “GC deferred”; o código tem `compact_vlog`. Não
  editei o RFC (é texto histórico P2.2). A cisão vive no LEDGER.

**Pendente (vai):** R006 Monkey.
