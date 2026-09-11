# RFC: 0198 — composição registrada e invariantes indutivos (o próximo degrau seL4-class)

**Status:** draft
**Updated:** 2026-09-10

## Background

- O RFC-0191 fechou TODAS as fatias P0/P1/P2 (2026-09-10, sweep P1.6 em
  `findings/2026-09-10-rfc0191-p16-sweep-final/`). Estado terminal da
  escada: extract 248 / close 2 / atom 31 / model 17; `cap_data_fate` 100
  (alvo ≤100 atingido); `floor_atom` 31 (meta ≥8 superada).
- Os boards de composição/concorrência/escala estão pagos NO LEAN
  (17/17 glue callers com unfold de plano E callee; 5/5 linhas de
  concorrência caller+callee; `unpaid_compose=0`, `unpaid_concurrency=0`,
  `unpaid_scale=0`) — mas esse crédito NÃO sobe a escada: `close_proofs.tsv`
  tem UM close registrado (`sift_step`) e os 5 módulos `Compose*.lean`
  (16 teoremas) não valem degrau.
- Os invariantes de produto são de UM PASSO: `wal_append_preserves_inv_wal`
  (Inv-WAL, P2.1) e `inv_lsm_newest_first_never_non_live` (Inv-LSM, P2.2)
  provam preservação por uma operação — não indução sobre estados
  alcançáveis, que é o formato seL4 (invariante de sistema + base inicial
  + todo passo preserva).
- Débito de script restante: `unpaid_script=2/17` — `finish_group_off_lock`
  e `group_finish` sem o token `write_pending_frame`. Ambos vivem em
  `crates/pedradb-core/src/concurrent.rs`, arquivo NÃO-commitado da sessão
  otimizar paralela — implementar fora da janela dela é editar o voo dela.
- Tier model com 17 pares stand-in (`scale_predict`, `scale_happy_hot`,
  `scale_forecast`, `leveling_*`, `probe_order_covering`, …): proof_depth
  `model`, sem teorema sobre N concreto.

## Problems This Solves

- **Problem 1 — composição não conta:** o degrau `close` da escada
  (RFC-0188) tem 1 registro em 298 pares; o trabalho compose já feito não
  move `floor_close`.
- **Problem 2 — um passo não é invariante:** sem base inicial e sem
  corolário de alcançabilidade, o lema de um passo não liga a uma garantia
  de estado ("qualquer estado alcançável pelo write path satisfaz
  Inv-WAL").
- **Problem 3 — script order incompleto no grupo:** dois handlers de
  group-commit ainda não pagam o token de ordem `write_pending_frame`
  (débito nomeado pelo board, bloqueado por coordenação).
- **Problem 4 — model twins:** 17 pares em stand-in; propriedades de
  leveling/escala sem número concreto por trás.

## Proposed Solution

- Registrar closes de glue como degrau: iff ∀ sobre o par plano∘callee
  (corpo dos dois, ∃-mold), linha `close` em `close_proofs.tsv` +
  `floor_close` +1 NO MESMO commit (a receita exata dos 30 passos do
  P2.3, mudando o kind).
- Subir os invariantes para a forma indutiva: base (estado inicial
  satisfaz) + corolário de alcançabilidade citando o lema um-passo já
  registrado — cada um um fire.
- Pagar os 2 tokens de script quando a sessão paralela landar o
  `concurrent.rs` (fatia de coordenação, não de agora).
- Graduar models em teoremas sobre N concreto (mesma régua do rank 10).

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)

- [x] **P0.1** Primeiro close de glue REGISTRADO: `wal_commit_plan ∘
  fence_on_sync_fail` — iff ∀ (existencial sobre os dois corpos
  extraídos) em `WriteAdmission.lean`; linha `close` em
  `close_proofs.tsv` + `floor_close` 1→2 + residuals no mesmo commit —
  status: `done` (theorem `wal_commit_plan_ok_iff_fence_chain`; floor
  247/2, residuals extract 247 / close 3)
- [ ] **P0.2** Segundo close registrado: `occ_batch_plan ∘ occ_conflict`
  (glue de `validate_occ_batch`/`lone_commit`) — iff ∀ em
  `GroupCommit.lean`; `floor_close` 2→3 no mesmo commit — status: `todo`

### P1 — next wave (depends on P0 or clearly deferrable)

- [x] **P1.1** Inv-WAL base indutiva: `inv_wal_init` (estado inicial
  satisfaz) + corolário `inv_wal_reachable` (todo estado alcançável por
  n appends satisfaz, citando `wal_append_preserves_inv_wal`) em
  `WalState.lean` — status: `done` (predicado indutivo
  `wal_append_reach`; passo = lema um-passo registrado, não re-provado)
- [x] **P1.2** Inv-WAL passo sync/fence: preservação pela classe de op
  sync/fence (fecha "todo passo do write path que toca o WAL preserva")
  — status: `done` (família indutiva `wal_write_step`; passo append
  cita `wal_append_preserves_inv_wal` do 0191, passos sync/ack são os
  lemas um-passo novos `wal_sync_preserves_inv_wal` /
  `wal_ack_preserves_inv_wal`; corolário
  `wal_write_step_preserves_inv_wal` cobre os três)
- [x] **P1.3** Inv-LSM corolário indutivo: cadeia de k merges preserva
  newest-first-never-non-live, citando `inv_lsm_newest_first_never_non_live`
  — status: `done` (estrutura `MergeStep` + premissas `merge_step_newest_first`
  / `merge_step_answers_live`; predicado indutivo `merge_chain` Nat-indexado
  (base = cadeia vazia); corolário `merge_chain_preserves_inv_lsm` por indução
  sobre a cadeia — o passo CITA o lema um-passo registrado, não re-prova)
- [ ] **P1.4** Tokens de script `write_pending_frame` em
  `finish_group_off_lock` + `group_finish` (COORDENAÇÃO: `concurrent.rs`
  pertence à sessão paralela; só landar depois do commit dela ou em
  janela acordada; bloqueio nomeado aqui, não silenciado) — status: `todo`

### P2 — later / polish

- [ ] **P2.1** Primeira graduação model→concreto: teorema sobre N
  concreto num `scale_kernel` (`scale_forecast`, callers/callees já
  unfoldados) + re-tier do par — status: `todo` (a graduação de scale é
  pague pelo [RFC-0199](0199-escada-de-contagem-complexidade-verificada.md)
  P0.3, crédito `count`; este item fecha com aquele)
- [ ] **P2.2** Cadência cap continua: cada novo atom df desce
  `cap_data_fate` 100→99→… no mesmo commit (mesma receita um-por-commit
  do P2.3/0191; sem alvo numérico novo além da monotonicidade) —
  status: `todo`
- [ ] **P2.3** Herdados seguem terminais: L28 user-gated, TCG/F_FULLFSYNC
  nightly, N=4 aberto (registro em `verification-ledger.md`
  §"Herdados do 0187" — este RFC NÃO re-abre; alargar fronteira continua
  sendo movimento de ledger) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Close de glue registrado: wal_commit_plan∘fence_on_sync_fail | done | wal_commit_plan_ok_iff_fence_chain | 2026-09-10 |
| P0.2 | p0 | Close de glue registrado: occ_batch_plan∘occ_conflict | todo | — | 2026-09-10 |
| P1.1 | p1 | Inv-WAL base + alcançabilidade | done | inv_wal_init + inv_wal_reachable | 2026-09-10 |
| P1.2 | p1 | Inv-WAL passo sync/fence | done | wal_write_step_preserves_inv_wal | 2026-09-11 |
| P1.3 | p1 | Inv-LSM corolário indutivo (k merges) | done | merge_chain_preserves_inv_lsm | 2026-09-11 |
| P1.4 | p1 | Tokens write_pending_frame (2 handlers, coordenação concurrent.rs) | todo | — | 2026-09-10 |
| P2.1 | p2 | Model→concreto: scale_kernel N concreto | todo | — | 2026-09-10 |
| P2.2 | p2 | Cadência cap monotônica (df atoms) | todo | — | 2026-09-10 |
| P2.3 | p2 | Herdados 0187 seguem terminais (sem re-abrir) | todo | — | 2026-09-10 |

## Acceptance Criteria

- **Tests:** cada close/lema registrado roda `scripts/lean_extracts.sh
  --required` verde e o `cargo test` nomeado que chama a fn de produção
  (planta do par); gates `depth-floor` + `product-floor` + `ledger`
  GREEN no commit de cada promoção.
- **Telemetria:** none — por quê: fatias de prova; os números vivem na
  escada (`proof_depth.tsv`/`residuals.json`), não em métricas de runtime.
- **Documentation:** checkbox + status table deste RFC no mesmo commit do
  slice; linha nova em `docs/verification-ledger.md` quando um close ou
  invariante muda a camada de uma garantia.
- **Screenshots:** backend-only (Lean/Rust gates).

## Out of scope

- Flipar `media_durable_admitted` / `forall_schedules_admitted` /
  `lock_interleavings_admitted` (seguem `ok false`; ∀π continua recusado).
- Dump de `db.rs`/`concurrent.rs`; extrair `concurrent.rs` inteiro.
- Cartoons Montanha (`montanha-fdb-recipes`, 4 twins) — user-gated.
- TCG power-cut / `F_FULLFSYNC` como teorema — sempre experimento.
- Re-abrir os herdados do 0187 (P2.4/0191 é o registro terminal).
- "Somos seL4" — não somos; o caminho continua por degraus registrados.
