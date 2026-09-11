# RFC: 0199 — Escada de contagem: complexidade verificada por operação

**Status:** draft
**Updated:** 2026-09-10
**ID:** 0199
**Parents:** [0176](0176-modelo-matematico-de-escala.md) (o modelo: \(P\) provado
em Verus, \(T\) medido), [0188](0188-segundo-degrau-close-atom.md) (a escada
extract→close→atom), [0191](0191-pacote-garantias-produto.md) (o pacote de
garantias e o cap `data_fate`), [0198](0198-composicao-registrada-invariantes-indutivos.md)
(invariantes indutivos — este RFC os consome como hipóteses),
[0192](0192-write-cycle-forecast.md) (o kernel de write cujas fatias ganham
multiplicadores provados aqui)
Nota de régua: cotas de trabalho **não são claim de perf**. Cartaz continua
sendo Pedra vs RocksDB default `sync=false` (`ROCKS_PARITY_SYNC=0`); nada aqui
vira win, e previsões continuam hat com erro nomeado.

> **Tese:** "escalabilidade linear" não é um enunciado que se prova; o que se
> prova é **cota de trabalho por operação em função do tamanho do dataset,
> dadas invariantes** — contagem de passos e de chamadas a primitivas nomeadas
> (`pwrite`, `fdatasync`, `pread`, …) sobre os extracts Aeneas do código real.
> Hoje essas contagens vivem como *twins* sem teorema: 17 pares `model`
> stand-in (`scale_predict`, `scale_happy_hot`, `scale_forecast`, `leveling_*`,
> `probe_order_covering`, …) e as fatias do `write_cycle_kernel` — fórmulas
> corretas, ∀ sobre o código inexistente. Este RFC abre a **escada de
> contagem**: um novo crédito (`count`) na régua do ratchet, um inventário
> vivo de kernels com medida de tamanho e cota-alvo, e a fronteira de
> ferramenta própria — inclusive para modelar `fdatasync` — quando a extração
> não der conta. É um programa aberto: cada fire paga uma cota, `floor_count`
> só sobe.

## Background

- **O que já está provado e em que nível.** RFC-0176: \(P\) (probes do point
  get) e \(\mathrm{cap}(R)\) provados **em Verus, no nível modelo**;
  \(T\) (ns) é âncora medida. O `scale_kernel.rs` (RFC-0176) e o
  `write_cycle_kernel.rs` (RFC-0192) são os twins GET/write — o
  `point_get_probes = levels + l0_covering` e a decomposição de ciclo
  (`publish 820 ns`, `mem_insert 580 ns`, …) são aritmética sobre contagens
  que **nenhum teorema liga ao código extraído** (0198: os 5 módulos
  `Compose*.lean` não valem degrau; os 17 pares scale são stand-in).
- **O enunciado certo (e os dois absolves).** (a) Scan completo \(O(n)\) é a
  cota *correta*, não regressão. (b) Custo local alto com \(n\) local
  limitado (memtable capped, L0 capped) vira amortizado por write — o que
  importa é a função do **dataset total**, com a amortização explícita
  (write-amplification clássica: Monkey/Dostoevsky/SILK, já fichados).
- **Estado da arte não resolve por nós.** Não existe ferramenta push-button
  de big-O para Rust arbitrário. Time credits (Coq/CFML/Iris) são
  por-algoritmo, em ML-like; AARA/TiML são linguagens restritas. O caminho
  realista é o que este RFC fatia: invariantes (0198) + lemas de contagem
  sobre os extracts + primitivas contáveis.
- **O buraco nomeado que uma cota teria pegado.** `P_as-is` walk-all
  (RFC-0176): o get que olha \(N_{\mathrm{files}}\) runs em vez de
  `levels + l0_max` — um invariantes-quebrada que apareceu no modelo, não no
  código, e que um teorema `count` sobre o `Lookup` real teria recusado no
  commit.
- **O que contagem formal não dá:** ns de relógio. O custo de um
  `fdatasync` no tempo é âncora datada, nunca teorema (herdado 0187). O que
  É formalizável: **quantos** `fdatasync`/`pwrite` o caminho confirmado
  executa por operação/grupo — o multiplicador que compõe com a âncora no
  `write_cycle_kernel`.

## Problems This Solves

- **Problem:** as fórmulas de escala/ciclo são twins sem ∀ sobre o código —
  "linear" é palpite até existir teorema contando o extract real.
- **Problem:** regressão de complexidade (loop que vira quadrático, ladder
  que vira walk-all) só aparece no meter/cartaz — tarde e caro; sem gate no
  commit.
- **Problem:** nenhuma prova diz quantas vezes cada fatia nomeada do
  write cycle (nem cada primitiva de IO) executa por op — o forecast
  compõe contagens implícitas.
- **Problem:** quando a primitiva é `fdatasync` (fora do modelo Aeneas),
  não existe caminho de formalização — falta álgebra de primitivas.

## Proposed Solution

1. **Novo crédito de escada: `count`.** Teorema sobre extract real da forma
   `∀ entrada (satisfazendo invariantes nomeadas), work f entrada ≤ g(size)`
   onde `work` conta passos de recursão e chamadas a primitivas nomeadas.
   Registro no ratchet com kind `count` + `floor_count` monotônico (mesma
   receita RFC-0188: linha no mesmo commit do teorema).
2. **Invariantes como hipóteses, contagem como consumidor.** O 0198 sobe os
   invariantes à forma indutiva; as cotas aqui os citam (razão de níveis,
   sortedness das runs, cap de L0/memtable) — sem duplicar o trabalho dele.
3. **Álgebra de primitivas IO no modelo Lean.** `pwrite`, `fdatasync`,
   `pread`, `fadvise`… como construtores contáveis (`Work.io`); teoremas
   contam chamadas por operação (ex.: ≤1 `fdatasync` por grupo confirmado).
   Constantes ns ficam nas âncoras datadas e compõem no `write_cycle_kernel`
   (formal dá o multiplicador, medido dá o termo).
4. **Ferramenta própria quando faltar.** Se a extração Aeneas não preservar
   a estrutura de custo (ou não modelar a primitiva), construímos a
   ferramenta no repo: tradução com anotação de custo (fork pinnado, regra
   PINS.md). Programado, não prometido a pronto.
5. **Inventário vivo + cadência.** Tabela kernel → medida de tamanho →
   cota-alvo → status (`model`/`count`); cada fire paga uma linha; o mapa
   expande indefinidamente — este RFC é o primeiro degrau de um programa
   aberto, não um pacote fechado.

## Delivery slices (mandatory)

### P0 — must ship first (primeira cota vertical)

- [x] **P0.1** Inventário vivo `docs/verification-ledger.md` §"Escada de
  contagem": kernel → medida de tamanho → cota-alvo → status. Primeiras
  linhas: point-get ladder (`Lookup`/`ProbeOrder`), merge/sift do compact
  (`Merge`/`Compact`), insert memtable, caminho de write confirmado
  (`WriteAdmission`/`GroupCommit`), bloom probe (`Bloom`), scan
  (`Scan`/`Iter`) — status: `done` (2026-09-11; kind `count` + `floor_count`
  aceitos no gate com selftest)
- [x] **P0.2** Kind `count` no ratchet (scripts aceitam; `floor_count = 0`→1
  no mesmo commit) + **primeiro teorema de contagem**: one-pass do walk de
  compact — sobre o extract `LsmR1Kernel` (`lsm_compact_src_loop`/
  `lsm_compact_inner_loop`; o walk de produção `WindowKvIter` é recusado
  pelo Charon), teorema `lsm_compact_work_bound` (twin `lsm_compact_src_steps`
  ≤ Σ entradas abaixo do nível-alvo + níveis drenados; pontes: `cont` ⇒
  índice +1 exato, `done (some _)` só após `src.len`, drain desce exato 1
  nível), com twin test Rust (`tests/lsm_compact_count.rs`: contador
  instrumentado no teste, mesmo bound) — status: `done` (2026-09-11)
- [ ] **P0.3** Cota do point-get ladder sobre `Lookup`/`ProbeOrder`:
  probes ≤ levels + l0_max dada a invariante de razão; gradua
  `probe_order_covering`/`scale_predict` de `model` → `count`
  (absorve a P2.1 do 0198 — cross-ref lá) — status: `todo`

### P1 — next wave (amortização e primitivas)

- [ ] **P1.1** Álgebra `Work.io` (pwrite/fdatasync/pread/fadvise como
  construtores contáveis no modelo Lean) + teorema: caminho de write
  confirmado executa ≤1 `fdatasync` por grupo — sobre os extracts
  `WriteAdmission`/`GroupCommit` — status: `todo`
- [ ] **P1.2** Amortização memtable→flush: trabalho total de flush por k
  writes ≤ c·k dado o cap (contagem com crédito; no mínimo N concreto,
  régua 0198) — status: `todo`
- [ ] **P1.3** Composição formal×medido: `write_cycle_kernel` consome as
  contagens provadas como multiplicadores × âncoras ns datadas — o forecast
  passa a citar os teoremas `count` (sem virar claim de cartaz) —
  status: `todo`
- [ ] **P1.4** Cota de scan/range: linear no resultado (uma passada por run
  dadas invariantes) sobre `Scan`/`Iter` — status: `todo`

### P2 — later / ferramenta própria e fronteira

- [ ] **P2.1** Cost-instrumented extract: se a mão não escalar, estender a
  tradução Aeneas (fork pinnado) para emitir funções com anotação de custo
  automática — ferramenta nossa, mantida no repo — status: `todo`
- [ ] **P2.2** Fronteira fdatasync no modelo: quando a extração não modelar
  a primitiva, modelá-la nós mesmos na álgebra `Work.io` (semântica de
  barreira contável; a âncora ns por classe de host — `F_FULLFSYNC` darwin
  vs `fdatasync` linux — continua medida e datada; nunca teorema de ns) —
  status: `todo`
- [ ] **P2.3** Cadência contínua: cada kernel novo do inventário ganha cota
  (um por fire); `floor_count` só sobe; inventário atualizado no mesmo
  commit — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Inventário vivo de kernels + cotas-alvo | done | 2026-09-11 | 2026-09-11 |
| P0.2 | p0 | Kind `count` + one-pass do walk de compact (teorema + twin test) | done | 2026-09-11 | 2026-09-11 |
| P0.3 | p0 | Point-get ladder ≤ levels + l0_max (model→count) | todo | — | 2026-09-10 |
| P1.1 | p1 | `Work.io` + ≤1 fdatasync por grupo confirmado | todo | — | 2026-09-10 |
| P1.2 | p1 | Amortização memtable→flush (k writes) | todo | — | 2026-09-10 |
| P1.3 | p1 | write_cycle compõe contagens provadas × âncoras | todo | — | 2026-09-10 |
| P1.4 | p1 | Scan linear no resultado | todo | — | 2026-09-10 |
| P2.1 | p2 | Cost-instrumented extract (ferramenta própria) | todo | — | 2026-09-10 |
| P2.2 | p2 | fdatasync modelado na álgebra (primitiva nossa) | todo | — | 2026-09-10 |
| P2.3 | p2 | Cadência: uma cota por fire, floor_count monotônico | todo | — | 2026-09-10 |

## Acceptance Criteria

- **Tests:** cada teorema `count` roda `scripts/lean_extracts.sh --required`
  verde **e** o twin test cargo nomeado (mesmo bound, contador instrumentado
  no teste, não em hot path de produção); gates `depth-floor` +
  `product-floor` + `ledger` GREEN no commit de cada cota; linha `count` no
  ratchet no mesmo commit.
- **Telemetry / Analytics:** none — por quê: fatias de prova; os números
  vivem no ratchet TSV e nas âncoras datadas, não em métricas de runtime.
- **Documentation:** checkbox + status table deste RFC e a linha do
  inventário no mesmo commit da cota; linha em `docs/verification-ledger.md`
  quando uma cota muda de camada (`model`→`count`).
- **Screenshots:** backend-only (Lean/Rust gates).

## Out of scope

- Teorema de tempo: ns de `fdatasync`/IO são âncoras medidas datadas,
  nunca teorema (herdado 0187; TCG power-cut / `F_FULLFSYNC` seguem
  experimento — não re-abrir).
- ∀ global "nenhum algoritmo é O(n²)" sobre o código todo — o alvo é cota
  por kernel do inventário, com o enunciado por operação/amortizado.
- Claim de perf/cartaz: cotas não são wins; a régua Rocks default
  `sync=false` segue intocada; `ratio_hat`/forecast seguem hats com erro
  nomeado (0197).
- Flipar `media_durable_admitted` / `forall_schedules_admitted` /
  `lock_interleavings_admitted` (seguem `ok false`; ∀π continua recusado —
  fronteira 0198).
- Meter 100M / host gate do 0197 — paralelos: não bloqueiam e não são
  bloqueados por este RFC.
