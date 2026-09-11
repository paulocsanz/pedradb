# RFC: 0202 — os quatro teoremas de concorrência: a faixa do ConcurrentDb que a campanha não prova

**Status:** draft
**Updated:** 2026-09-11
**Parents:** [0200](0200-alcancabilidade-completa-write-path-merge.md)
(alcançabilidade completa + cadência atom; fechou 6/6),
[0198](0198-composicao-registrada-invariantes-indutivos.md) (composição
registrada + escada close→atom),
[0191](0191-pacote-garantias-produto.md) (lemas um-passo e data-fate)

Nota de régua: fatias de prova; nenhum claim de perf. Cartaz continua
sendo Pedra vs RocksDB default `sync=false` (`ROCKS_PARITY_SYNC=0`).

> **Tese:** o write path agora tem a frase seL4 completa por PASSO
> (0200: qualquer sequência de passos satisfaz Inv-WAL; merge com base e
> ponte de estrutura) — mas o `ConcurrentDb` multi-thread ainda é
> coberto por CAMPANHA (PCT d=3/d=4, exaustivo N≤3, TSan), não por
> teorema. O mapa de concorrência
> (`findings/2026-09-07-concurrency-proof-map/`) nomeia QUATRO
> propriedades, e o board vivo mostra o que cada uma ainda deve:
> escada no fechamento do 0200 em `d3a1c517` — extract 245 / close
> registrados 4 (floor 4) / atom 33 (floor 33) / count 6 /
> `cap_data_fate` 98 / compose 17/17 pago (`unpaid_compose=0/17`,
> `unpaid_script=0/17`, `unpaid_concurrency=0/5`). Este RFC paga cada
> fila da tabela de concorrência no degrau que o extract suporta — um
> por commit, régua mecânica herdada — e REGISTRA a fila do
> escalonador como recusa (teorema de que a claim é recusada), não como
> prova de ∀π.

## Background

- **As quatro filas** (referência
  `.grok/skills/caminho-sel4/references/concurrency.md`):
  1. **Data race (UB)** — Safe Rust AXM + protocolo do CLIENTE
     write-lock: `rwlock_client_may_mutate` (kernel
     `group_commit_kernel.rs`, handler `finish_group_off_lock`;
     mutação de `Db` só sob guarda). Par `data_fate` NÃO pago (um dos
     98). Extrato existe (`GroupCommitKernel.lean`).
  2. **Lost-update** — `occ_conflict`/`group_validate` pagos no passo
     (close 0198 `occ_batch_plan∘occ_conflict`), o caller link
     `validate_occ_batch`/`lone_commit` nomeado nos handlers; falta o
     destino do MEMBRO: `occ_member_fate` (TooOld > Conflict > Ok),
     par `data_fate` não pago, extrato existe.
  3. **Deadlock** — `wait_for_deadlock` (2PL locktab,
     `crates/rocksdb-compat/src/locktab.rs`): extrato existe
     (`LocktabKernel.lean`) MAS o corpo é um loop sobre HashMap/HashSet
     com lookups opacos — o núcleo provável é a PONTE do corpo do loop
     com hipóteses de lookup (molde do re-escopo datado do 0200 P1.2),
     não o iff completo do ciclo sem semântica de mapa.
  4. **Escalonador ∀π** — `lock_interleavings_admitted` segue `ok
     false` (teorema de recusa já registrado em produção:
     `claim_lock_interleavings_refused_after_put`). Esta fila se paga
     REGISTRANDO a recusa no RFC, nunca flipando.
- **Vizinhança:** a campanha (R-pct/R-group-glue em
  `scripts/formal/residuals.json`) continua sendo a evidência
  empírica; este RFC não a duplica — paga o degrau formal que falta em
  cada fila.

## Problems This Solves

- **Problem:** "sem data race" hoje é Safe Rust + campanha; o
  protocolo do cliente (mutação só sob guarda) é um par `data_fate`
  sem teorema registrado — um leitor honesto não pode citar a
  propriedade.
- **Problem:** o lost-update tem o plano do grupo provado
  (`occ_batch_plan`) mas o destino do membro individual
  (`occ_member_fate`) segue `data_fate` — a fila N-way do mapa cita os
  dois.
- **Problem:** a fila deadlock não tem NENHUM teorema sobre o extrato
  do detector — nem ponte, nem dente as-is.
- **Problem:** a fila do escalonador corre risco de ser lida como
  "faltando" quando na verdade é uma recusa deliberada — precisa estar
  escrita como entrega.

## Proposed Solution

1. **P0.1:** `rwlock_client_may_mutate` a atom (fila data-race):
   forall-iff sobre o corpo extraído — mutação permitida exatamente
   quando o cliente SEGURA a write-lock; AS-IS (muta sem guarda) fica
   inalcançável pelo iff. `cap_data_fate` 98→97, `floor_atom` 33→34,
   atom row + catalog flip + residuals no MESMO commit.
2. **P0.2:** `occ_member_fate` a atom (fila lost-update, caller link
   `validate_occ_batch`): forall-iff — TooOld vence sobre Conflict
   vence sobre Ok; AS-IS nunca-aborta inalcançável. `cap` 97→96,
   `floor_atom` 34→35, mesmo commit.
3. **P1.1:** fila deadlock: ponte do `wait_for_deadlock` — teorema
   sobre o CORPO do loop extraído com hipóteses de lookup do
   wait-for graph (+ dente as-is sempre-false: detector desligado).
   Se o Charon/Aeneas medir recusa no corpo, registra a recusa em
   `formal/aeneas/EXTRACT.md` com re-escopo datado e cai para o
   próximo atom df — cadência não trava (padrão 0200 P1.2).
4. **P1.2:** quinto close registrado (`floor_close` 4→5): candidato
   nomeado `group_validate` (o passo N-way do lost-update: cada
   elemento do output É `occ_conflict` do membro i contra `last_seq`;
   indução no loop extraído, premissa par-local). Se o corpo medir
   recusa (slices/loop Aeneas), cai para o próximo par do board com
   corpo tratável — um close por commit, registro no mesmo commit.
5. **P2:** cadência contínua (atoms 96→…, um por commit), fila do
   escalonador REGISTRADA como recusa (linha do RFC citando
   `lock_interleavings_admitted ok false` e o teorema de recusa de
   produção), sweep final de gates.

## Delivery slices (mandatory)

### P0 — must ship first (as duas filas de cliente que o extract já suporta)

- [x] **P0.1** `rwlock_client_may_mutate` a atom (fila data-race,
  write-lock client protocol; cap 98→97, floor_atom 33→34 no mesmo
  commit) — status: `done` (teorema
  `rwlock_client_may_mutate_ok_iff_holding_write` em GroupCommit.lean:
  mutação de `Db` permitida EXATAMENTE enquanto o cliente segura a
  write-guard — o AS-IS mutar-após-soltar é inalcançável; build verde
  primeira tentativa, sorry 0; planta DST existente
  `rwlock_client_may_mutate_on_live_off_lock_is_not_ok` no kernel real)
- [x] **P0.2** `occ_member_fate` a atom (fila lost-update; handler
  `validate_occ_batch`; cap 97→96, floor_atom 34→35 no mesmo commit) —
  status: `done` (teorema `occ_member_fate_ok_iff_precedence` em
  GroupCommit.lean: o destino do membro OCC é EXATAMENTE a precedência
  TooOld > Conflict > Ok; o AS-IS nunca-abortar (membro lagando comita)
  é inalcançável; build verde primeira tentativa, sorry 0; planta DST
  existente `occ_member_fate_on_live_conflict_is_not_ok` no kernel real)

### P1 — next wave (deadlock e o quinto close)

- [x] **P1.1** Ponte `wait_for_deadlock` (corpo do loop + hipóteses de
  lookup + dente as-is) OU recusa medida com re-escopo datado —
  status: `done` (ponte PAGA: as três arestas de saída de um passo do
  detector em Locktab.lean — `wait_for_deadlock_step_nowait_is_alive`
  (espera de ninguém ⇒ `done false`), `wait_for_deadlock_step_cycle_closes`
  (cadeia fecha no waiter ⇒ `done true`), `wait_for_deadlock_step_revisit_reports_cycle`
  (revisit ⇒ `done true`) — com hipóteses de lookup fixando as
  chamadas-axioma; par promovido a atom (cap 96→95, floor_atom 35→36,
  floor_extract 243→242); fronteira datada em
  `formal/aeneas/EXTRACT.md`: o iff completo do ciclo precisa de
  semântica de mapa (HashMap.get/insert são axiomas no Aeneas) e fica
  TCB; dente as-is já existia (`wait_for_deadlock_as_is_dente`);
  build Locktab verde, sorry 0)
- [ ] **P1.2** Quinto close registrado (candidato `group_validate`
  N-way; senão próximo par do board; floor_close 4→5 no mesmo commit) —
  status: `todo`

### P2 — later / cadência + recusa registrada + sweep

- [ ] **P2.1** Fila do escalonador registrada como RECUSA
  (`lock_interleavings_admitted` segue `ok false`; nenhum flip) +
  cadência atoms contínua — status: `todo`
- [ ] **P2.2** Sweep: gates GREEN no HEAD, zero sorry nos wrappers
  tocados, capturas — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Fila data-race: atom rwlock_client_may_mutate | done | rwlock_client_may_mutate_ok_iff_holding_write (GroupCommit.lean) | 2026-09-11 |
| P0.2 | p0 | Fila lost-update: atom occ_member_fate | done | occ_member_fate_ok_iff_precedence (GroupCommit.lean) | 2026-09-11 |
| P1.1 | p1 | Fila deadlock: ponte wait_for_deadlock (ou recusa datada) | done | wait_for_deadlock_step_{nowait_is_alive,cycle_closes,revisit_reports_cycle} (Locktab.lean) + fronteira EXTRACT.md | 2026-09-11 |
| P1.2 | p1 | Quinto close registrado (candidato group_validate) | todo | — | 2026-09-11 |
| P2.1 | p2 | Escalonador: recusa registrada + cadência atoms | todo | — | 2026-09-11 |
| P2.2 | p2 | Sweep final de gates | todo | — | 2026-09-11 |

## Acceptance Criteria

- **Tests:** cada atom/close roda `scripts/lean_extracts.sh --required`
  verde; gates `depth-floor` + `product-floor` + `ledger` GREEN no
  commit de cada promoção; registro (TSV/floors/residuals/catalog) no
  mesmo commit; planta DST do par dirigindo o kernel real passa.
- **Telemetry:** none — fatias de prova; números vivem na escada.
- **Documentation:** checkbox + status table no mesmo commit do slice;
  recusa medida (se houver) em `formal/aeneas/EXTRACT.md` com data.
- **Screenshots:** backend-only.

## Out of scope

- Flipar `media_durable_admitted` / `forall_schedules_admitted` /
  `lock_interleavings_admitted` (seguem `ok false`; a fila do
  escalonador é uma RECUSA registrada, não uma prova de ∀π).
- Claim "somos seL4" ou equivalência; prova ∀π de interleavings de
  `ConcurrentDb`; semântica completa de HashMap no deadlock (o que não
  for provável do extract fica no re-escopo datado).
- Cotas de trabalho (0199); campanhas L28/PCT (user-gated); cartoons
  Montanha (user-gated); herdados 0187 (terminais); claim de perf.
