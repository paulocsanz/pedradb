# RFC: 0200 — alcançabilidade completa: estados do write path e saídas de merge (o degrau indutivo seguinte)

**Status:** draft
**Updated:** 2026-09-11
**Parents:** [0191](0191-pacote-garantias-produto.md) (lemas um-passo),
[0198](0198-composicao-registrada-invariantes-indutivos.md) (composição
registrada + invariantes indutivos),
[0199](0199-escada-de-contagem-complexidade-verificada.md) (escada de
contagem — consome invariantes como hipóteses)

Nota de régua: fatias de prova; nenhum claim de perf. Cartaz continua
sendo Pedra vs RocksDB default `sync=false` (`ROCKS_PARITY_SYNC=0`).

> **Tese:** o 0198 deixou a forma indutiva pela metade. Inv-WAL tem base
> inicial e alcançabilidade só para CADEIAS DE APPEND
> (`wal_append_reach`), e o corolário de classe
> (`wal_write_step_preserves_inv_wal`) cobre um passo de CADA tipo — mas
> NÃO existe o teorema que fecha a frase seL4 completa: "todo estado
> alcançável por QUALQUER sequência de passos do write path (append, sync,
> fence, ack, em qualquer ordem) satisfaz Inv-WAL". Inv-LSM tem a cadeia
> de k merges (`merge_chain`), mas a cadeia não nasce de uma base de
> saída inicial nem liga ao kernel de estrutura (`sift_step`) que restaura
> o heap newest-first a cada saída. Este RFC fecha essas duas pontas e
> registra o próximo close de glue — a escada continua um degrau por
> commit.

## Background

- **O que o 0198 landou (estado terminal daquele RFC):** closes
  registrados 2→3 (`wal_commit_plan ∘ fence_on_sync_fail`,
  `occ_batch_plan ∘ occ_conflict`; floor_close 3); Inv-WAL: `inv_wal_init`
  (base) + `inv_wal_reachable` (cadeias de append) +
  `wal_sync_preserves_inv_wal` / `wal_ack_preserves_inv_wal` (passos
  novos) + `wal_write_step_preserves_inv_wal` (corolário da classe);
  Inv-LSM: `merge_chain_preserves_inv_lsm` (cadeia de k merges citando o
  lema um-passo registrado); tokens `write_pending_frame` pagos (board
  `unpaid_script=0/17`); primeira descida de cap do ciclo
  (`occ_snap_uses_published` a atom, `cap_data_fate` 100→99). Escada:
  extract 246 / close 3 / atom 32 / count 1.
- **O que falta para a frase completa (as duas pontas):**
  1. Alcançabilidade da CLASSE: `wal_write_step_reach` — cadeias de
     passos de QUALQUER construtor da família a partir do estado inicial.
     Hoje só append-conta; a física real do WAL é append→sync→ack
     intercalado.
  2. Base da cadeia de merges: `merge_chain` hoje é lista de passos com
     premissa estrutural por passo; falta a forma "saída alcançável" —
     toda saída que o merge produz a partir da saída vazia, com o heap
     restaurado pelo kernel de decisão (`sift_step`) a cada passo.
- **Vizinhança:** o 0199 (escada de contagem) declara que suas cotas
  CONSOMEM as invariantes do 0198 como hipóteses (razão de níveis,
  sortedness, cap de L0). Este RFC não duplica cotas — fornece as pontes
  que as hipóteses deles precisam citar.

## Problems This Solves

- **Problem:** "todo estado alcançável satisfaz Inv-WAL" é verdade só
  para append-contas — um leitor honesto não pode citar a garantia para
  schedules com sync/ack intercalados, que é o caso real do group
  commit.
- **Problem:** `merge_chain` sem base de saída inicial é um lema sobre
  listas bem-formadas, não sobre o que o compact REAL produz.
- **Problem:** o crédito `close` (3 registrados) ainda deixa o board
  compose (16 teoremas) sem degrau — a cadência precisa do próximo par.

## Proposed Solution

1. **P0.1:** família indutiva `wal_write_step_reach : Nat → WalState →
   Prop` (cadeias de passos de qualquer construtor de
   `wal_write_step` a partir de `wal_state_init`) + corolário
   `inv_wal_write_reachable`: indução na cadeia, base =
   `inv_wal_init`, passo = CITAR `wal_write_step_preserves_inv_wal`
   (registrado no 0198) — nada re-provado.
2. **P0.2:** terceiro→quarto close de glue registrado (floor_close
   3→4): próximo par do board compose com corpo extraído tratável, iff
   ∀ no molde ∃ dos dois primeiros.
3. **P1.1:** base de saída para o merge: `merge_output_reach` (saídas
   produzidas a partir da saída vazia, cada passo newest-first pela
   premissa estrutural já definida) + corolário compondo com
   `merge_chain_preserves_inv_lsm`.
4. **P1.2:** ponte estrutura↔cadeia: o passo de restauração do heap é o
   kernel `sift_step` (0188, primeiro close registrado) — lema de que a
   premissa estrutural da cadeia é mantida pelo sift (o Stay não é
   reparo, o Swap escolhe o melhor filho; fecha com
   `merge_sift_step_repairs_iff`).
5. **P2:** cadência contínua — cap desce por atom df, floor_close sobe
   por close, um por commit; L28 segue user-gated; herdados 0187
   seguem terminais.

## Delivery slices (mandatory)

### P0 — must ship first (a frase completa do WAL + o próximo close)

- [x] **P0.1** `wal_write_step_reach` + `inv_wal_write_reachable` em
  `WalState.lean` — corolário por indução citando `inv_wal_init` (base)
  e `wal_write_step_preserves_inv_wal` (passo, registrado 0198) —
  status: `done` (família indutiva Nat-indexada sobre QUALQUER
  construtor de `wal_write_step` a partir de `wal_state_init`; build
  verde primeira tentativa, sorry 0)
- [x] **P0.2** Quarto close de glue registrado (par do board compose a
  escolher pelo corpo tratável; iff ∀ ∃-mold, floor_close 3→4 no mesmo
  commit) — status: `done` (par `wal_rotate_decision ∘
  wal_segment_is_empty` — o passo do caller real `try_rotate_wal`
  (db.rs): decisão, recheck de inflight sob a mutex, segmento vazio
  pula; teorema `try_rotate_step_rotates_iff_pins_clear_segment_live`
  em `Flush.lean`: o passo dispara `rotate_wal_now` EXATAMENTE quando
  decisão = RotateWal ∧ recheck idle ∧ segmento COM dados (nunca
  reescreve MANIFEST ociosamente); catalog pair novo
  `wal_rotate_decision` (298→299) + linha close + floor_close 3→4 +
  residuals close 5 no MESMO commit; build Flush verde, sorry 0)

### P1 — next wave (a base do merge e a ponte de estrutura)

- [x] **P1.1** `merge_output_reach` + corolário da saída alcançável
  (composição com `merge_chain_preserves_inv_lsm`) em `Merge.lean` —
  status: `done` (saída em ordem de EMISSÃO a partir da vazia, um passo
  por `emit`; ponte `merge_output_reach_chain`: emissão lida de trás
  pra frente É cadeia `merge_chain`; corolário compõe ponte +
  corolário da cadeia — duas citações, zero re-prova)
- [x] **P1.2** Ponte sift↔newest-first: a premissa estrutural da cadeia
  sobrevive ao `sift_step` (cita `merge_sift_step_repairs_iff`) —
  status: `done` (camada `TaggedSift` em `Merge.lean`: a decisão `s` É
  a do kernel (`tagged_kernel_decision`); `tagged_step_stays_iff_no_repair`
  re-exporta o close registrado 0188 Stay↔não-reparo;
  `merge_step_newest_first_congr` — a premissa é LOCAL ao par (não lê
  kind/range_hidden); `tagged_stay_preserves_newest_first` +
  `tagged_stay_extends_chain` — o Stay preserva/estende a cadeia por
  par; `tagged_repair_kernel_moves_as_is_stays` — em reparo o kernel
  move e o as-is fica (com `merge_sift_step_as_is_stays_on_repair`).
  **RE-ESCOPO DATADO 2026-09-11**: "o Swap restaura newest-first"
  não é provável do extract — o comparador é axioma
  (`CoreCmpPartialOrdShared0B.lt`) e o sift extraído não carrega
  estado de heap (só os três bools); a ponte cobre o núcleo provável
  (Stay = não-reparo preserva a premissa por par; divergência as-is
  em reparo). Build verde, sorry 0, zero re-prova)

### P2 — later / cadência

- [x] **P2.1** Cadência: cada novo atom df desce cap (99→…), cada novo
  close sobe floor_close, um por commit (régua mecânica herdada) —
  status: `done` (primeiro atom df do ciclo: par `wal_sync_required`
  (write_admission_kernel.rs, handler `commit_ops_with`) pago em
  `wal_sync_required_ok_iff_client_else_db` (WriteAdmission.lean) — a
  decisão de `fdatasync` por commit é single-valued: a escolha
  EXPLÍCITA do cliente quando ele setou `sync`, senão o default do db;
  iff ∀ sobre o corpo extraído, build verde sorry 0; cap_data_fate
  99→98, floor_atom 32→33, floor_extract 246→245, residuals no MESMO
  commit; planta DST existente
  `wal_sync_required_on_live_client_true_is_not_ok` dirigindo o kernel
  real)
- [ ] **P2.2** Sweep: todos os gates GREEN no HEAD, zero sorry nos
  wrappers tocados, capturas — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Alcançabilidade da classe write-path (Inv-WAL) | done | inv_wal_write_reachable | 2026-09-11 |
| P0.2 | p0 | Quarto close de glue registrado | done | try_rotate_step_rotates_iff_pins_clear_segment_live (Flush.lean) | 2026-09-11 |
| P1.1 | p1 | Base de saída do merge + corolário alcançável | done | merge_output_reach_preserves_inv_lsm | 2026-09-11 |
| P1.2 | p1 | Ponte sift_step↔newest-first | done | TaggedSift + tagged_step_stays_iff_no_repair + tagged_stay_extends_chain | 2026-09-11 |
| P2.1 | p2 | Cadência cap/floor_close contínua | done | wal_sync_required_ok_iff_client_else_db (WriteAdmission.lean) | 2026-09-11 |
| P2.2 | p2 | Sweep final de gates | todo | — | 2026-09-11 |

## Acceptance Criteria

- **Tests:** cada lema/close roda `scripts/lean_extracts.sh --required`
  verde; gates `depth-floor` + `product-floor` + `ledger` GREEN no
  commit de cada promoção; registro (TSV/residuals) no mesmo commit.
- **Telemetry:** none — fatias de prova; números vivem na escada.
- **Documentation:** checkbox + status table no mesmo commit do slice;
  linha em `docs/verification-ledger.md` quando um close muda camada de
  garantia.
- **Screenshots:** backend-only.

## Out of scope

- Cotas de trabalho (são do 0199; este RFC só fornece as pontes que
  elas citam).
- Flipar `media_durable_admitted` / `forall_schedules_admitted` /
  `lock_interleavings_admitted` (seguem `ok false`).
- Cartoons Montanha (user-gated); herdados 0187 (terminais);
  claim de perf; "somos seL4".
