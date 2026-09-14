# RFC-0222: Escada até o par seL4 — gap por eixo com denominador nomeado, medido por máquina

**Status:** active (2026-09-14: P0.1–P0.6 done; P0.7 wip 2/8 `client_axis` + `group_window`)
**Updated:** 2026-09-13

## Background

- A [auditoria independente de 2026-09-13](../audits/2026-09-13-formal-verification-independent-audit.md) respondeu a pergunta "somos par do seL4?" com números re-derivados: **mesma classe de claim** (prova relativa a TCB escrito, residual publicado, "sem bugs" proibida — RFC-0061), **classe de garantia inferior**. Nenhum número do audit foi contestado; todos os que eu re-executei confirmaram (ledger RED era meu — pago em `8c605f6b`; 4 scripts Verus, 3 barreiras e 12 caller-lints já eram vermelhos no parent pré-objetivo `df11b725`).
- A mesma realidade dá **percentuais diferentes por denominador** (audit §2): 92,95% de degraus ∀ sobre pares do catálogo; 96,5% dos proof-tier verdes; **17,3% do src das crates formalizadas é kernel** (28.464/164.420); dentro dos kernels, 93,8% dos pub fns na superfície provada (804/857) e 61/69 arquivos enrolados. Discussão sem denominador nomeado é otimismo ou marketing — o cânone do repo proíbe os dois.
- O [RFC-0221](0221-piso-verde-24h-o-que-agentes-compram-do-sel4.md) (draft paralelo) cobre o **Piso Verde em 24h** como experimento de fan-out. Este RFC define o **destino** — o gap por eixo até o par, com métrica de máquina — e executa os terminais que não dependem de fan-out. As fatias P0 dos dois RFCs convergem nos mesmos gates (o gate verde É a definição de feito; trabalho idempotente, sem conflito de arquivo).
- O [RFC-0220](0220-escada-de-composicao-encadeia-os-atomos-dual-unfold.md) é o mecanismo do eixo 1 (composição → teorema topo). Este RFC não duplica as fatias dele; mede o eixo.

## Problems This Solves

- **Problem:** "igual ao seL4" não tem métrica de máquina — cada discussão re-deriva números na mão (o auditor precisou de find/grep/wc por horas). Sem métrica, o alvo deriva para o denominador mais lisonjeiro.
- **Problem:** a árvore está vermelha nos próprios gates (barrier 3 sítios, 98 clocks falso-positivos, coverage-map 3 semanas defasado, 4 scripts Verus mortos, 12 caller-lints) — vermelho herdado, mas vermelho.
- **Problem:** nenhum eixo tem piso — a superfície provada pode regredir (kernels crescendo sem proof) sem nenhum gate acusar.

## Proposed Solution

`scripts/sel4_gap.py`: mede **cada eixo do audit §6 mecanicamente, com denominador nomeado**, e imprime a tabela (atual → destino). Modo `--gate` congela pisos por eixo em `scripts/ratchet/sel4_gap_floors.json` (nunca regredir; subir = mover o piso no mesmo commit da prova). Os dois compostos do audit ficam definidos no script:

- **Bloco classe-de-claim+evidência** (eixos 8–10 do audit + estado dos gates): hoje ~35%; destino 100% = todos os gates baratos verdes + CI verde no GitHub + TCB 100% nomeado.
- **Bloco que define o seL4** (eixos 1–4, 6–7): hoje ~15–20%; teto de engenharia (RFC + semanas-meses) ~60–70% — espinha topo sobre o write-path, superfície 100% dos kernels, espinha de recovery; o resto (confinamento, ∀-concorrência do `ConcurrentDb` inteiro, translation validation do binário) é **pesquisa nomeada com terminal honesto**, igual ao seL4 que pagou ~20 pessoas-ano.

Frase de venda permitida (cânone): *"programa de verificação na classe de claim seL4/IronFleet"* — nunca "par do seL4", nunca "sem bugs".

## Delivery slices (mandatory)

### P0 — métrica + piso verde (cada fatia = 1 commit, gates verdes no mesmo commit)

- [x] **P0.1** `scripts/sel4_gap.py` + baseline datado em findings + modo `--gate` com pisos por eixo — status: done (`11cb107c`)
- [x] **P0.2** barrier floor: 3 sítios (db.rs sync_dir 13→15, wal/mod.rs sync_data 1→2, fullfsync_anchor sync_all 0→2) com `--crash-log` no mesmo commit (a amarração dinâmica que a regra manda) — status: done (`73399698`)
- [x] **P0.3** clock gate: falso-positivo do harness — `three_teeth_queued.rs` é `#[cfg(test)]`-gated no `lib.rs`; o gate aprende a honrar gating de módulo no pai (não exempt-list: correção do heurístico documentado no próprio docstring) — status: done (`2b37ef71`)
- [x] **P0.4** coverage-map.md re-medido (69 kernels/28.464 LOC/312 pares; a regra própria do mapa) + os 4 sorries da stdlib Aeneas nomeados na tabela TCB do ledger — status: done (re-medição por máquina `sel4_gap.py` 68 kernels/27.960 LOC; sorries verificados no pin `daa85d7`; junto com P0.5 neste commit)
- [x] **P0.5** 4 scripts Verus: `vote_decision`/`dictionary_link` (arquivos perderam o bloco `verus!` — restaura in-file twin ou migra a rota do par para Aeneas onde o teorema já existe) e `changelog_rebuild`/`lookup` (lemmas chamam fns exec no `ensures` — restatear em spec/`when_used_as_spec`) — status: done (2 runners órfãos deletados + 3 pares migrados p/ Aeneas; 2 runners reparados verde: 13/14 verified 0 errors)
- [x] **P0.6** 12 caller-lints: cada um ou a produção volta a chamar o kernel ou o catalog aponta o caller real (trampolim pós-0219) — status: done (6 pares / 11 linhas FAIL: `run_disjoint` code-side — `disjoint_sorted_by_lo` chama `run_pairwise_disjoint_los`; `range_covers`/`flush_plan`/`probe_order_covering`/`group_validate`/`occ_member_fate` catalog-side — caller real pós-0219)
- [ ] **P0.7** onda de enrollment (8 kernels sem rota + ~53 pub fns fora da superfície): kernel+twin+script no padrão dos 73 — fan-out, converge com 0221 P0.6 — status: `wip` (2/8: `client_axis_kernel`, `group_window_kernel`)
- [ ] **P0.8** push: `proof-check` + `verification-gates` verdes no GitHub (portão do usuário: o push é dele) — status: `todo`

### P1 — pisos e CI (semanas)

- [ ] **P1.1** `proof-check.yml` com elan+lean+charon+aeneas pinados por sha; `lean_extracts.sh --required` + 73× `aeneas_*.sh --required` no job (mata o skip→exit 0) — converge com 0221 P1.1 — status: `todo`
- [ ] **P1.2** pisos do `sel4_gap --gate` ligados ao CI (regredir eixo = red) — status: `todo`

### P2 — os terminais de pesquisa (nomeados, incrementais; meses+)

- [ ] **P2.1** teorema topo de refinamento pela escada do RFC-0220 (cada fatia 0220 move o eixo 1 aqui) — status: `todo`
- [ ] **P2.2** espinha de recovery: os átomos de wal_recover/manifest/reopen/vlog numa única composição boot-estabelece-invariante (eixo 4 do audit) — status: `todo`
- [ ] **P2.3** confinamento/integridade (2º teorema do seL4) — status: `todo`
- [ ] **P2.4** redução de concorrência do `ConcurrentDb` via kernel de group-commit — status: `todo`
- [ ] **P2.5** translation validation de rustc/LLVM pinado num alvo (RFC-0171 P2) — status: `todo`

## Risks / non-goals

- **Non-goal:** declarar paridade seL4. O destino honesto deste RFC é o teto de engenharia (~60–70% do bloco definidor) + terminais de pesquisa nomeados com residual publicado.
- Os pisos por eixo medem o que a árvore sustenta hoje; denominador de LOC inclui `#[cfg(test)]` inline (regra atual do repo — mudar a regra é movimento de ledger, não de script).
- P0.7 (enrollment) é fan-out com merge serial (catalog/ledger/floors movem no mesmo commit — o gargalo de Amdahl que o 0221 registra); este RFC não tenta serializar a onda inteira numa sessão.
- Eixo CI (P0.8) depende de push — portão do usuário, não do agente.

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | sel4_gap.py + baseline + --gate | done | `11cb107c` | 2026-09-13 |
| P0.2 | p0 | barrier floor 3 sítios + crash-log | done | `73399698` | 2026-09-13 |
| P0.3 | p0 | clock gate honra gating de módulo | done | `2b37ef71` | 2026-09-13 |
| P0.4 | p0 | coverage-map re-medido + 4 sorries no TCB | done | este commit | 2026-09-13 |
| P0.5 | p0 | 4 scripts Verus consertados | done | este commit | 2026-09-13 |
| P0.6 | p0 | 12 caller-lints fechados | done | este commit | 2026-09-14 |
| P0.7 | p0 | onda de enrollment (fan-out) | wip 2/8 | este commit | 2026-09-14 |
| P0.8 | p0 | CI verde no GitHub (push) | todo | — | 2026-09-13 |
| P1.1 | p1 | toolchains formais no proof-check.yml | todo | — | 2026-09-13 |
| P1.2 | p1 | pisos sel4_gap no CI | todo | — | 2026-09-13 |
| P2.1 | p2 | teorema topo (via RFC-0220) | todo | — | 2026-09-13 |
| P2.2 | p2 | espinha de recovery | todo | — | 2026-09-13 |
| P2.3 | p2 | confinamento/integridade | todo | — | 2026-09-13 |
| P2.4 | p2 | redução de concorrência | todo | — | 2026-09-13 |
| P2.5 | p2 | translation validation do binário | todo | — | 2026-09-13 |
