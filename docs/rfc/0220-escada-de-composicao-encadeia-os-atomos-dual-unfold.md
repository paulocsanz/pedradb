# RFC-0220: escada de composição — encadear os átomos em espinhas ∀ (dual-unfold)

**Status:** in-progress (P0–P1 + P2.1–P2.3 paid; P2.4 sweep resta)
**Updated:** 2026-09-16

## Contexto

O degrau extrato foi drenado (RFC-0218: 62,33% → 92,47%) e o degrau
trampolim também (RFC-0219: 92,47% → **290/312 = 92,95%**; pool pagável
saturado — os 22 pendentes são 10 cartoon Montanha no portão do usuário
+ 12 cânone-excluídos por construção). Os 286 átomos registrados são
**ilhas**: cada um prova um iff-∀ sobre UM kernel; nenhum teorema
encadeia duas decisões adjacentes do caminho de escrita.

Medido no board vivo (2026-09-13, HEAD `b67e8cc3`):

- teoremas de composição existentes: 32, em 11 libs `Compose*.lean`;
- átomos nomeados em ao menos uma composição: **33/286 = 11,54%**;
- da família nascida no RFC-0218/0219 (13 kernels de fate), só
  `wal_commit_plan` aparece em composição — `flusher_gate_plan`,
  `parked_debt_plan`, `changelog_durable_commit_fate`,
  `fence_admission_plan`, `manifest_publish_plan`, `cf_flush_plan`,
  `auto_flush_gate`, `mem_auto_flush_plan`, `changelog_store_plan`,
  `pit_resync_rewrite_plan`, `parked_pop_plan`, `group_ack_plan`
  nunca foram encadeados.

A estrutura classe-seL4 é composição: propriedades provadas sobre
módulos encadeadas em teoremas de refinamento. Ilhas isoladas não
compoem nada sozinhas. O caminho do destino de uma escrita —
`worker anexado? → dívida real? → sync contado? → sync falhou cerca? →
SST durável publica?` — é provado peça a peça; a CADEIA inteira não é
provada em lugar nenhum.

## Problemas que resolve

- **Problema:** o fate do writer é ∀ peça a peça, nunca em cadeia —
  composições existentes (32) cobrem 11,54% dos átomos e nenhuma
  família do 0218/0219.
- **Problema:** a régua de composição não existe — ninguém mede quais
  átomos estão encadeados; uma regressão silenciosa (composição
  quebrada por re-extração) não fecha gate.
- **Problema:** o board de concorrência (`references/concurrency.md`)
  tem linhas não pagas: o caller de cola (`validate_occ_batch`,
  `lone_commit`, `finish_group_off_lock` — unfold só do callee não
  paga), N-way do `group_validate`, data-race row (lock-order vs
  flush rotate), deadlock 2PL, PCT d=2 nomeado.

## Solução proposta

Escada de composição: teoremas cross-lib `Compose*.lean` com
**dual-unfold** (caller-plan E callee, entrada representativa + ramo
as-is — `references/aeneas.md`), rito RFC-0219 adaptado (1 composição
= 1 commit, teorema + `lake build` + registro). Cada composição
registra linha `close` no par existente (precedente RFC-0200
`try_rotate_step_rotates_iff_pins_clear_segment_live`) — o par já é
átomo, conta uma vez; a composição PAGA em profundidade, não em
numerador.

Métricas registradas (ambas datadas, monotônicas):

- **m1** = `sel4_coverage` (não regredir; só sobe com decisão nova
  real — nunca por composição, que não muda numerador por construção);
- **m2** = razão de composição = átomos nomeados em ≥ 1 teorema
  `Compose*.lean` ÷ `floor_atom`, medida por
  `scripts/check_compose_floor.py` (novo gate, piso congela o valor).

## Fatias de entrega

### P0 — espinha do fate do writer (menor fatia vertical útil)

- [x] **P0.1** gate `check_compose_floor.py` (+ `--selftest`, job
  `compose-floor`), piso m2 honesto (RFC-0227) — status: `done`
- [x] **P0.2** `ComposeWriter.lean`: cadeia workerless —
  `flusher_gate_plan` × `parked_debt_plan` (sem worker NADA parqueia,
  inclusive com dívida no cap; ramo as-is: workerless dorme) — status: done (RFC-0222 P2.1, 2026-09-14)
- [x] **P0.3** cadeia do sync — `changelog_durable_commit_fate` ×
  `wal_commit_plan` (Count com sync exige AppendSync; Skip async só
  AppendApplyOk; as-is cerca nada) — status: done (RFC-0224 P0.1, 2026-09-14)
- [x] **P0.4** cadeia do fence — `wal_commit_plan::AppendSyncFence` ×
  `fence_admission_plan` (sync requerido falhou ⇒ cerca recusa TUDO
  depois; as-is admite) — status: done (RFC-0224 P0.2, 2026-09-14)
- [x] **P0.5** cadeia da publicação — `manifest_publish_plan` ×
  `changelog_durable_commit_fate` (SST durável + commit contado ⇒
  publica; hold fail-closed senão) — status: done (RFC-0224 P0.3, 2026-09-14)
- [x] **P0.6** sweep P0 em worktree + nota datada em
  `formal/aeneas/EXTRACT.md` — status: `done` (RFC-0227 EXTRACT 2026-09-15)

### P1 — espinhas de flush, lookup e grupo

- [x] **P1.1** espinha de auto-flush: `auto_flush_gate` ×
  `mem_auto_flush_plan` × `cf_flush_plan` (scan pula ⇒ família pula;
  eixo acima ⇒ família due flusha) — status: `done` (RFC-0227 P2.4 `ComposeProduct.lean`)
- [x] **P1.2** espinha OCC/lookup: `occ_snap_lock_order` ×
  `occ_snap_uses_published` + exemplo N-way `group_validate` (N>2,
  membro lagging conflita — paga o caller de cola por dual-unfold do
  plano que o handler chama) — status: `done` (RFC-0227 P2.3)
- [x] **P1.3** espinha do grupo: `parked_pop_plan` × `group_ack_plan`
  (par válido popeia; ack solo cerca no io-fail) — status: `done` (RFC-0227 P2.4)
- [x] **P1.4** espinha do changelog: `changelog_store_plan` ×
  `pit_resync_rewrite_plan` — status: `done` (RFC-0227 P2.4)
- [x] **P1.5** sweep P1 + nota datada — status: `done` (RFC-0227 EXTRACT 2026-09-15)

### P2 — board de concorrência terminal + sweep final

- [x] **P2.1** data-race row: lock-order vs flush rotate (compose
  `occ_snap_lock_order` × `wal_rotate_decision` com `commit_inflight`)
  — status: `done` (RFC-0227 P2.1)
- [x] **P2.2** deadlock row: `wait_for_deadlock` 2PL locktab (teorema
  do ciclo, não LockBud) — status: `done` (RFC-0227 P2.2)
- [x] **P2.3** scheduler row: PCT d=2 nomeado como PLANTA (campanha ≠
  ∀π fica no TCB — nunca promovida a teorema) — status: `done`
  (absorvido: [RFC-0229](0229-host-tcb-media-sched-pct-stdenv-liveness.md) P0.3;
  `pct_chain3_row_is_plant`; `planted_chain3_found_by_pct_d3`)
- [ ] **P2.4** sweep final em worktree DENTRO de `software/` (gates
  verdes, sorry 0, `lean_extracts.sh --required` exit 0) + EXTRACT.md
  + `**Status:** done` — status: `todo`

## Status (vivo — atualizar no mesmo commit do código)

| ID | Banda | Título | Status | Task / PR | Updated |
|----|-------|--------|--------|-----------|---------|
| P0.1 | p0 | gate compose-floor + selftest | done | RFC-0227 | 2026-09-15 |
| P0.2 | p0 | cadeia workerless (flusher_gate × parked_debt) | done | RFC-0222 P2.1 | 2026-09-14 |
| P0.3 | p0 | cadeia do sync (fate × wal_commit_plan) | done | RFC-0224 P0.1 | 2026-09-14 |
| P0.4 | p0 | cadeia do fence (wal_commit × fence_admission) | done | RFC-0224 P0.2 | 2026-09-14 |
| P0.5 | p0 | cadeia da publicação (manifest_publish × fate) | done | RFC-0224 P0.3 | 2026-09-14 |
| P0.6 | p0 | sweep P0 + EXTRACT.md | done | RFC-0227 EXTRACT | 2026-09-15 |
| P1.1 | p1 | espinha auto-flush ×3 | done | RFC-0227 ComposeProduct | 2026-09-15 |
| P1.2 | p1 | espinha OCC/lookup + N-way | done | RFC-0227 P2.3 | 2026-09-15 |
| P1.3 | p1 | espinha do grupo (parked_pop × group_ack) | done | RFC-0227 ComposeProduct | 2026-09-15 |
| P1.4 | p1 | espinha changelog × pit_resync | done | RFC-0227 ComposeProduct | 2026-09-15 |
| P1.5 | p1 | sweep P1 | done | RFC-0227 EXTRACT | 2026-09-15 |
| P2.1 | p2 | data-race row (lock-order × rotate) | done | RFC-0227 P2.1 | 2026-09-15 |
| P2.2 | p2 | deadlock row (2PL ciclo) | done | RFC-0227 P2.2 | 2026-09-15 |
| P2.3 | p2 | scheduler row (PCT d=2 planta nomeada) | done | RFC-0229 P0.3 | 2026-09-16 |
| P2.4 | p2 | sweep final + done | todo | — | 2026-09-13 |

## Meta mensurável (alvos datados)

- **m2 (razão de composição)**: baseline 33/286 = **11,54%**
  (2026-09-13). P0 (2026-10-15): ≥ 45/286 = **15,73%** (≥ 12 átomos
  encadeados — as 4 cadeias de P0 + vizinhos já compostos recontados).
  P1 (2026-11-30): ≥ 65/286 = **22,73%**. P2 (2026-12-31): ≥
  80/286 = **27,97%** + board de concorrência terminal.
- **m1 (`sel4_coverage`)**: não regredir de **290/312 = 92,95%**
  (piso monotônio); sobe somente com par novo real (decisão nova de
  kernel), nunca por composição (linha `close` em par já coberto
  conta uma vez — precedente RFC-0200).
- Cadência: 1 composição = 1 commit (teorema dual-unfold + `lake
  build` verde + linha `close` no TSV + planta quando o caller é
  código vivo). Recusa medida nomeada vale (finding datado) — igual
  ao rito 0219.

## Critérios de aceite

- **Composição paga** = dual-unfold do caller-plan E do callee
  (`references/aeneas.md`), entrada representativa + ramo as-is;
  `native_decide` sem `unfold` NÃO é compose; unfold só do callee NÃO
  paga o caller de cola (`references/concurrency.md`).
- Cada cadeia fecha com o teorema encadeado verde em `lake build` e a
  linha `close catalog:<id>` no `scripts/ratchet/close_proofs.tsv`.
- Gate `check_compose_floor.py` verde em todo commit do RFC (piso
  m2 monotônico); `scripts/lean_extracts.sh --required` exit 0 no
  sweep; sorry 0 sempre.
- Campanha nunca vira ∀π (PCT d=2 é planta nomeada, não teorema);
  Montanha cartoon e os 12 cânone-excluídos seguem nos seus portões
  (usuário / decisão registrada de cânone) — promoção silenciosa é
  vermelho.
- NÃO é declaração de paridade seL4.

## Fora de escopo

- Levantar o cartoon Montanha (10 pares) — portão do usuário.
- Os 12 cânone-excluídos — só por decisão registrada de cânone.
- `db_rs_extracted` (extrair db.rs inteiro) — cânone do 0218 mantém.
- Campanha/Darwin-meter (RFC-0217 board) — paralelo, não este RFC.

## Vereditos / riscos

- Cross-lib pode lake-red por discriminante de instância duplicado
  (medido: T1Modelo+Txn) — desdobrar a cópia shim e restatear o
  callee na própria lib, nomeado no `EXTRACT.md`
  (`references/aeneas.md`).
- Composição de loop (`partial_fixpoint`) precisa `LawfulBEq` +
  `native_decide` em exemplos — não `rfl` do loop inteiro.
- Se uma cadeia revelar decisão de cola não-isolável (espalhada por
  locks/epochs), a recusa medida por sítio com número publicado é o
  terminal honesto — igual ao P2.1/P2.2 do 0219.
- A % m1 pode NÃO se mover no RFC inteiro (composição não muda
  numerador) — o avanço drástico é estrutural (m2 + cadeias) e é
  reportado como tal, sem inflar número.
