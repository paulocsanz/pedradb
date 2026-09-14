# RFC-0221: Piso Verde em 24h — o que a paralelização de agentes compra do caminho seL4 (e o que nenhum exército de agentes compra em 1 dia)

**Status:** draft
**Updated:** 2026-09-13

## Background

- A auditoria independente de 2026-09-13 ([docs/audits/2026-09-13-formal-verification-independent-audit.md](../audits/2026-09-13-formal-verification-independent-audit.md)) mediu o estado real: 312 pares (286 proof / 26 campaign), 68 kernels / 27,9k LOC = 17% do src das crates formalizadas, Lean próprio 85,6k LOC com 0 sorry no corpus construído — e, no dia, **a árvore vermelha nos próprios gates**: `pedra_formal.py --ci` exit 1 (140 FAIL: 107 enrollment, 12 caller-lint reais, 8 tcb-freeze, 8 three-teeth, 1 Lean, 4 metadata), 4/18 scripts Verus falhando, gates ledger/barrier/no-prod-time vermelhos, **7/7 runs de `proof-check` no GitHub = failure**, Lean/Aeneas fora do CI, e 335 commits à frente da origem.
- RFC-0220 mediu a composição: 32 teoremas `Compose*`, **33/286 = 11,54% dos átomos encadeados** — os átomos são ilhas. A estrutura classe-seL4 é a composição (espinhas ∀ → refinamento topo), não a contagem de ilhas.
- A pergunta que motivou este RFC: *"chegamos no seL4 em 1 dia? É possível, basta paralelizar o suficiente em agentes."*

**Resposta curta (a tese deste RFC): NÃO — e o RFC prova os dois lados com números.** O inventário paralelizável do caminho seL4 (~175 tarefas independentes de 1–3h-agente ≈ 20–35 dias-agente) cabe em **1 dia de 20–30 agentes** — mas isso compra o **Piso Verde** (tudo que o repo *alega* verde, verde e evidenciado externamente), não o par. O que falta para o par é serial por construção: a espinha de composição do RFC-0220 até UM teorema topo, a redução de concorrência, a prova de confinamento e a validação de binário. seL4 pagou ~20 pessoas-ano por ~10k CLOC ([RFC-0061](0061-residuals-sel4-ironfleet.md) L15); nenhum número de agentes converte trabalho serial-dependente em paralelo.

## Problems This Solves

- **Problem:** a pergunta "1 dia?" fica respondida por otimismo ou por recusa; nunca por um plano com contador. Este RFC transforma a disputa em **experimento falsificável de 24h com scoreboard** (P0: ou fica verde em 24h, ou aprendemos exatamente onde a serialização morde).
- **Problem:** o repo tem corpus real mas **vermelho e sem evidência externa** — todo claim formal hoje depende de confiar no host do autor.
- **Problem:** agentes paralelos são alocados em tarefas não-independentes (catálogo/ledger/floors são arquivos únicos com regra de movimento no mesmo commit) e colidem; falta a topologia de fan-out correta.

## Proposed Solution

- **P0 = "Piso Verde em 24h"**: onda de agentes em worktrees, cada fatia ≤ 1 sessão de agente, merge serial por um integrador (a serialização honesta: `catalog.json` + marker do ledger + floors TSV movem no mesmo commit — a merge queue É o gargalo de Amdahl, e este RFC a planeja em vez de fingir que não existe).
- **P1 = a compra dura a semana**: Lean/Aeneas/Charon pinados no `proof-check.yml` (fim do skip→exit 0), espinhas de composição começando pelos 12 átomos de fate nunca encadeados, cold-start reproduzível.
- **P2 = o resto do seL4 (serial, semanas–meses)**: refinamento topo via escada 0220, redução de concorrência, confinamento, validação de binário. Fora do "1 dia" por hipótese; dentro do caminho.

A resposta "é possível em 1 dia?" fica registrada como: **Piso Verde sim (P0, ~90% de confiança em 24h com 20–30 agentes); par seL4 não (P2 tem dependência serial que fan-out não remove — estimativa 6–12 semanas de foco após o Piso Verde, com provas de composição aceitas no Lean).**

## Delivery slices (mandatory)

### P0 — Piso Verde em 24h (fan-out; ordem de merge = ordem da lista)

- [ ] **P0.1** Marker do ledger re-pinado aos números vivos do gate (`check_ledger_consistency.py` GREEN) — status: `todo`
- [ ] **P0.2** 4 scripts Verus consertados (vote/dictionary_link: rota podre → import vstd ou migra par para rota Aeneas; changelog_rebuild/lookup: drift exec-mode → prova da assinatura atual) — status: `todo`
- [ ] **P0.3** 12 quebras de caller-lint fechadas (`db.rs`/`concurrent.rs` voltam a chamar `probe_order_covering`, `group_validate`, `occ_member_fate`, …) — status: `todo`
- [ ] **P0.4** 3 sítios de barreira registrados em `barrier_sites.tsv` com teste de injeção no mesmo commit — status: `todo`
- [ ] **P0.5** Dentes faltando: 98 clocks do `three_teeth_queued.rs` (exempt de harness ou conserto), 8 three-teeth Montanha, `Properties.lean` — status: `todo`
- [ ] **P0.6** 115 kernels não-enrolados (107 residuals + 8 tcb-freeze) registram kernel+twin+script no padrão dos 73 existentes — status: `todo`
- [ ] **P0.7** `coverage-map.md` re-medido + linha TCB nomeando os 4 sorries da stdlib Aeneas — status: `todo`
- [ ] **P0.8** Push: `proof-check` + `verification-gates` verdes no GitHub (a definição de Piso Verde) — status: `todo`

### P1 — a compra dura a semana (depende de P0)

- [ ] **P1.1** `proof-check.yml` ganha elan+lean+charon+aeneas pinados por sha; roda `lean_extracts.sh --required` + 73× `aeneas_*.sh --required` — status: `todo`
- [ ] **P1.2** 12 átomos de fate da família 0218/0219 encadeados nas espinhas `Compose*` (primeira fila do RFC-0220) — status: `todo`
- [ ] **P1.3** Cold-start reproduzível: `scripts/setup_formal_toolchain.sh` + auditoria re-executada por um agente sem cache de host — status: `todo`

### P2 — o resto do seL4 (serial; fora do "1 dia" por construção)

- [ ] **P2.1** Teorema topo de refinamento ("implementação ⊨ spec dicionário+crash") pela escada do [RFC-0220](0220-escada-de-composicao-encadeia-os-atomos-dual-unfold.md) — status: `todo`
- [ ] **P2.2** Redução de concorrência do `ConcurrentDb` via o kernel de group-commit — status: `todo`
- [ ] **P2.3** Teorema de confinamento/integridade (o 2º teorema do seL4) — status: `todo`
- [ ] **P2.4** Translation validation de um rustc/LLVM pinado num alvo ([RFC-0171](0171-pagar-o-preco-sel4.md) P2) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Marker do ledger re-pinado (gate GREEN) | todo | — | 2026-09-13 |
| P0.2 | p0 | 4 scripts Verus consertados | todo | — | 2026-09-13 |
| P0.3 | p0 | 12 caller-lint fechados | todo | — | 2026-09-13 |
| P0.4 | p0 | 3 barreiras + injeção no mesmo commit | todo | — | 2026-09-13 |
| P0.5 | p0 | Dentes: clocks/three-teeth/Properties | todo | — | 2026-09-13 |
| P0.6 | p0 | 115 kernels enrolados | todo | — | 2026-09-13 |
| P0.7 | p0 | coverage-map re-medido + TCB sorries | todo | — | 2026-09-13 |
| P0.8 | p0 | CI verde no GitHub (Piso Verde) | todo | — | 2026-09-13 |
| P1.1 | p1 | Lean/Aeneas pinados no proof-check | todo | — | 2026-09-13 |
| P1.2 | p1 | 12 átomos de fate encadeados | todo | — | 2026-09-13 |
| P1.3 | p1 | Cold-start reproduzível | todo | — | 2026-09-13 |
| P2.1 | p2 | Teorema topo de refinamento | todo | — | 2026-09-13 |
| P2.2 | p2 | Redução de concorrência | todo | — | 2026-09-13 |
| P2.3 | p2 | Confinamento/integridade | todo | — | 2026-09-13 |
| P2.4 | p2 | Validação de binário | todo | — | 2026-09-13 |

## Acceptance Criteria

- **Tests:** os gates SÃO os testes — cada fatia P0 termina com o gate nomeado GREEN (`check_ledger_consistency.py`, os 4 `verus_*.sh` exit 0, `pedra_formal.py --ci` exit 0 com 0 FAIL, `check_barrier_floor.py` GREEN, `check_no_prod_time_spawn.py` 0 violações, `gh run list` com `proof-check` e `verification-gates` conclusion=success). P1.1: job Lean verde com `--required` (sem skip) no CI. P1.2: contagem de átomos encadeados no board do RFC-0220 sai de 33/286.
- **Telemetry / Analytics:** none — os gates e o board 0220 são a telemetria; sem métricas novas.
- **Documentation:** `coverage-map.md` re-medido (P0.7), tabela TCB do ledger com a stdlib Aeneas, este RFC atualizado no mesmo commit de cada fatia.
- **Screenshots:** none — backend/proofs only.

## Out of scope

- Provar SO/rustc/LLVM/Z3/Charon/Aeneas/Lean ou mídia (TCB `never` do [RFC-0061](0061-residuals-sel4-ironfleet.md) permanece).
- Reescrever o sistema em Isabelle/Dafny.
- A frase "par do seL4" / "tão robusto quanto seL4" / "sem bugs" — proibida pelo RFC-0061 até P2.1 existir; a frase permitida após P0.8 é *"todo claim formal é machine-checked, verde e reproduzível por um cético em ~10 minutos"*.
- P2 dentro da janela de 24h — o ponto inteiro deste RFC é não prometer isso.

## A aposta falsificável (scoreboard de 24h)

Se às H+24 da onda P0 o `--ci` não estiver 0-FAIL com CI verde, a hipótese "só paralelizar" fica refutada no próprio inventário paralelizável — e o RFC pede o post-mortem nomeando a serialização que mordeu. Se estiver, o que se comprou é o Piso Verde, e a distância ao par passa a ser inteira e exclusivamente o P2 (serial): estimativa honesta 6–12 semanas de foco com a escada 0220, não 1 dia.
