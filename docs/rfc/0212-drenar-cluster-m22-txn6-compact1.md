# RFC-0212 — drenar o bloco cluster: membership ×22 + txn ×6 + compact ×1

**Status:** draft

> Renumerada de 0211 para 0212 em 2026-09-11: colisão com a
> `0211-escalonamento-rmw-mc4-drenar-grupo.md` (dela, a262f095) —
> renumera-se a minha, nunca a dela.

## Tese

O bloco l28 está DRENADO (0210: 22 atoms + 7 fantasmas
aposentados; ZERO `data_fate` no `l28.rs`). O próximo bloco
coerente é o CLUSTER in-process — as decisões de fila que o 0210
pagou em TCP REAL deixa pendentes na forma queued: **22 pares em
`crates/pedradb-raft/src/membership_kernel.rs`**, **6 em
`crates/pedradb-store/src/txn_kernel.rs`** e **1 singleton
`compact_unleft`** (`compact_kernel.rs`) — os 29 nomeados no
EXTRACT.md datado de 2026-09-11. Drenar este bloco leva o catálogo
ao estado em que TODO `data_fate` pendente restante mora em kernels
de storage (write_admission 9, lookup 4, flush 3, cf 2, leveling 2
+ 6 singletons = 26 nomeados) — nenhum par de protocolo de cluster
fica sem veredito.

Números vivos no HEAD do 0210 (`48db12b4`): 292 pares (266 proof /
26 campaign), `cap_data_fate` 55, `floor_atom` 69,
`floor_extract` 209, close 6, count 7. Se os 29 virarem atoms:
**cap 55→26, floor_atom 69→98, floor_extract 209→180**. Corpos
REAIS (não pure-lifts): cada promoção é um iff sobre o extract
Aeneas do corpo de produção — o molde é o da cadência membership do
0208 (`removed_steps_down_fate_iff`, `joint_still_active_fate_iff`
em `Membership.lean`), com planta DST verde ANTES do commit e
veredito datado para qualquer par que meça ausente (fn/planta
inexistente ⇒ aposentadoria com recusa — nunca gate inventado; o
pool honesto manda sobre a meta numérica).

## Fatias

### P0 — core

1. **P0.1:** cadência membership 1/6 (discard) — `discard_uncommitted`,
   `discard_leader`, `drop_preimages`, `force_clear` (0143/0144/0139/0138;
   wrapper `Membership.lean`; plantas DST queued verdes): cap 55→51,
   floor_atom 69→73, floor_extract 209→205 — status: `doing`

   — 1/4 `done`: `discard_node_counts_fate_iff` (Membership.lean;
   discard vivo do sufixo não-commitado roda em TODA réplica local
   exatamente quando o nó é local — `ids` não é o portão), cap
   55→54, floor_atom 69→70, floor_extract 209→208; planta DST
   verde (`discard_node_counts_on_live_queued_is_not_ok`)

   — 2/4 `done`: `discard_leader_local_fate_iff` (Membership.lean;
   persist-leader do discard sem líder é nó LOCAL exatamente quando
   o nó escolhido é local — o repair de `next_index` roda onde o
   persist aterra), cap 54→53, floor_atom 70→71,
   floor_extract 208→207; planta DST verde
   (`discard_leader_local_on_live_queued_is_not_ok`)

   — 3/4 `done`: `drop_preimages_node_counts_fate_iff`
   (Membership.lean; preimages de prepare caem em TODA réplica
   local exatamente quando o nó é local — `ids` não é o portão),
   cap 53→52, floor_atom 71→72, floor_extract 207→206; planta DST
   verde (`drop_preimages_node_counts_on_live_queued_is_not_ok`)

   — 4/4 `done` (FECHAMENTO): `force_clear_node_counts_fate_iff`
   (Membership.lean; clear TX force-local roda em TODA réplica
   local exatamente quando o nó é local — `ids` não é o portão),
   cap 52→51, floor_atom 72→73, floor_extract 206→205; planta DST
   verde (`force_clear_node_counts_on_live_queued_is_not_ok`) —
   P0.1 fechado nos números exatos: cap 55→51, floor_atom 69→73,
   floor_extract 209→205 — status: `done`
2. **P0.2:** cadência membership 2/6 (persist+hint) — `persist_meta`,
   `persist_hist`, `persist_fence`, `hint_member` (0135/0136/0137/0146):
   cap 51→47, floor_atom 73→77, floor_extract 205→201 — status: `todo`

### P1 — next

3. **P1.1:** cadências membership 3/6 + 4/6 — `recover_apply_node`,
   `recover_truncate`, `recover_abort`, `identity_before_applied`
   (recover+identity); `open_peer_disk`, `local_id_member`,
   `reader_local`, `participating_member` (open/reader): ×8, cap
   47→39, floor_atom 77→85, floor_extract 201→193 — status: `todo`
4. **P1.2:** cadências membership 5/6 + 6/6 — `pending_joint_node`,
   `joint_target`, `joint_add_target`, `joint_leave_ok` (joint);
   `drop_repl_slot`, `drop_sent_through` (slot): ×6, cap 39→33,
   floor_atom 85→91, floor_extract 193→187 — membership ZERO
   `data_fate` medido ao vivo — status: `todo`
5. **P1.3:** veredito datado dos medidos ausentes — SE alguma
   promoção acima medir fn/planta/handler inexistente, veredito por
   par em findings (reescrever para fn viva SE o caminho existir em
   produção; senão aposentadoria com recusa datada), espelhos/
   âncoras do 0203/0204 verdes antes/depois — status: `todo`

### P2 — later / polish

6. **P2.1:** cadência final do bloco — txn ×6 (`discard_cut`,
   `leftover_txn_is_aborted`, `next_txn_id_after`,
   `prepare_error_aborts_earlier`, `reserve_si_gen`,
   `unreserve_si_gen`; wrapper `StoreTxn.lean`) + `compact_unleft`
   (wrapper `StoreCompact.lean`): ×7, cap 33→26, floor_atom
   91→98, floor_extract 187→180 — bloco cluster ZERO `data_fate`
   medido ao vivo — status: `todo`
7. **P2.2:** composição ∀ do protocolo de fim-de-fila queued
   (finish: discard-leader local ∧ persist fence/hist conforme o
   fate) sobre atoms registrados em nova compose lib (zero
   buracos; twins DST verdes; razão de SEM registro em findings —
   não é par único) + sweep final (worktree destacado DENTRO de
   `software/`, gates 3× GREEN, extracts ok, sorry 0, capturas em
   findings, nota datada em EXTRACT.md: cluster drenado; 26 de
   storage restantes nomeados) + flip `**Status:** done` —
   status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Cadência membership 1/6 — discard ×4 | done | 365e7c65 + e5c4c346 + 322b2241 + este commit (4 atoms, 4 commits; números exatos) | 2026-09-11 |
| P0.2 | p0 | Cadência membership 2/6 — persist+hint ×4 | todo | — | 2026-09-11 |
| P1.1 | p1 | Cadências membership 3/6+4/6 — recover+open ×8 | todo | — | 2026-09-11 |
| P1.2 | p1 | Cadências membership 5/6+6/6 — joint+slot ×6; ZERO data_fate | todo | — | 2026-09-11 |
| P1.3 | p1 | Veredito datado dos medidos ausentes | todo | — | 2026-09-11 |
| P2.1 | p2 | Cadência final — txn ×6 + compact ×1; cluster ZERO | todo | — | 2026-09-11 |
| P2.2 | p2 | Composição ∀ fim-de-fila + sweep final + flip done | todo | — | 2026-09-11 |

## Critérios de aceite

1. **Uma promoção = um commit** (teorema iff no wrapper Lean +
   linha `atom` no `close_proofs.tsv` + cirurgia de catálogo com
   `atom_reason` datado + flip de status/tabela NO MESMO COMMIT +
   findings próprio). Corpo REAL: o iff fala sobre o extract do
   corpo que o rustc liga — o pagamento é o teorema, não a
   contagem.
2. **Planta DST verde ANTES do commit** (ou imediatamente após,
   com revert se vermelha — o incidente trunc não se repete).
   Plantas em paralelo, nunca um for sequencial.
3. **Gates 3× GREEN a cada commit** (`check_depth_floor`,
   `check_product_floor`, `check_ledger_consistency`).
4. **Veredito antes de forçar**: par medido ausente ⇒ recusa datada
   em findings + cirurgia coerente (catálogo/glue/ledger/cap) —
   nunca gate de identidade inventado para agradar o catálogo.
5. **Composição sem registro TSV** — razão datada em findings
   (composição atravessa vários atoms, não é par único).
6. **Sweep no worktree destacado DENTRO de `software/`** — gates
   3× GREEN no HEAD final, `lean_extracts.sh --required` ok, sorry
   0 nos wrappers tocados, worktree removido após a captura.
7. As três admissions (`media_durable_admitted`,
   `forall_schedules_admitted`, `lock_interleavings_admitted`)
   seguem ALWAYS false — nunca flipadas.

## Out of scope

- Equivalência seL4 — nunca claim; o caminho é ratchet de proofs
  sobre corpos de produção.
- Perf/cartaz (RocksDB parity segue o peer default `sync=false`;
  sem claim novo neste RFC).
- WAL staging da RFC-0209 ( dela), âncoras host-timing do
  0203/0204, kernels write_cycle/scan_readahead/leftover_page/
  ratio_curve e seus pares aeneas_* — inalterados.
- Os 26 de storage (write_admission 9, lookup 4, flush 3, cf 2,
  leveling 2 + 6 singletons) — fica para a sucessora, nomeados no
  EXTRACT.md.
