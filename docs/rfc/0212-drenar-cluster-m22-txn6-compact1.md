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
   cap 51→47, floor_atom 73→77, floor_extract 205→201 — status: `done`

   — 1/4 `done`: `persist_meta_node_counts_fate_iff`
   (Membership.lean; meta SI persiste em TODA réplica local
   exatamente quando o nó é local — `ids` não é o portão), cap
   51→50, floor_atom 73→74, floor_extract 205→204; planta DST
   verde (`persist_meta_node_counts_on_live_queued_is_not_ok`,
   1 passed via --lib)

   — 2/4 `done`: `persist_hist_node_counts_fate_iff`
   (Membership.lean; hist SI persiste em TODA réplica local
   exatamente quando o nó é local), cap 50→49, floor_atom 74→75,
   floor_extract 204→203; planta DST verde
   (`persist_hist_node_counts_on_live_queued_is_not_ok`, 1 passed
   via --lib)

   — 3/4 `done`: `persist_fence_node_counts_fate_iff`
   (Membership.lean; cerca de aborto persiste em TODA réplica
   local exatamente quando o nó é local), cap 49→48,
   floor_atom 75→76, floor_extract 203→202; planta DST verde
   (`persist_fence_node_counts_on_live_queued_is_not_ok`, 1
   passed via --lib)

   — 4/4 `done` (FECHAMENTO): `hint_if_member_fate_iff`
   (Membership.lean; hint de roteamento do líder conta
   exatamente quando o nó apontado está em `ids` — um
   `leader_id` fora da membresia não é hint), cap 48→47,
   floor_atom 76→77, floor_extract 202→201; planta DST verde
   (`hint_if_member_on_live_queued_is_not_ok`, 1 passed via
   --lib) — P0.2 fechado nos números exatos: cap 51→47,
   floor_atom 73→77, floor_extract 205→201 — status: `done`

### P1 — next

3. **P1.1:** cadências membership 3/6 + 4/6 — `recover_apply_node`,
   `recover_truncate`, `recover_abort`, `identity_before_applied`
   (recover+identity); `open_peer_disk`, `local_id_member`,
   `reader_local`, `participating_member` (open/reader): ×8, cap
   47→39, floor_atom 77→85, floor_extract 201→193 — status: `done`

   — 1/8 `done`: `recover_apply_node_counts_fate_iff`
   (Membership.lean; recover aplica em TODA réplica local
   exatamente quando o nó é local — `ids` não é o portão), cap
   47→46, floor_atom 77→78, floor_extract 201→200; planta DST
   verde (`recover_apply_node_counts_on_live_queued_is_not_ok`,
   8 plantas em paralelo, 1.93s)

   — 2/8 `done`: `recover_truncate_node_counts_fate_iff`
   (Membership.lean; log truncado no recover persiste em TODA
   réplica local exatamente quando o nó é local), cap 46→45,
   floor_atom 78→79, floor_extract 200→199; planta DST verde
   (`recover_truncate_node_counts_on_live_queued_is_not_ok`,
   no mesmo lote paralelo)

   — 3/8 `done`: `recover_abort_node_counts_fate_iff`
   (Membership.lean; leftover 2PC é abortado no recover em TODA
   réplica local exatamente quando o nó é local), cap 45→44,
   floor_atom 79→80, floor_extract 199→198; planta DST verde
   (`recover_abort_node_counts_on_live_queued_is_not_ok`,
   no mesmo lote paralelo)

   — 4/8 `done`: `membership_identity_before_applied_fate_iff`
   (Membership.lean; identidade C-new persiste exatamente quando
   o persist de identidade vem ANTES de avançar applied passado
   o joint), cap 44→43, floor_atom 80→81, floor_extract 198→197;
   planta DST verde
   (`membership_identity_before_applied_on_live_queued_is_not_ok`,
   no mesmo lote paralelo)

   — 5/8 `done`: `open_peer_uses_disk_fate_iff`
   (Membership.lean; open in-process carrega peers do disco de
   membership, exatamente quando há membership em disco — nunca
   do CLI `n_nodes`), cap 43→42, floor_atom 81→82,
   floor_extract 197→196; planta DST verde
   (`open_peer_uses_disk_on_live_queued_is_not_ok`,
   no mesmo lote paralelo)

   — 6/8 `done`: `local_id_if_member_fate_iff`
   (Membership.lean; nó local único é identidade deste processo
   exatamente quando está em `ids`), cap 42→41, floor_atom
   82→83, floor_extract 196→195; planta DST verde
   (`local_id_if_member_on_live_queued_is_not_ok`,
   no mesmo lote paralelo)

   — 7/8 `done`: `reader_id_local_fate_iff`
   (Membership.lean; fallback `ids.first()` do LocalApplied é nó
   local exatamente quando é local), cap 41→40, floor_atom
   83→84, floor_extract 195→194; planta DST verde
   (`reader_id_local_on_live_queued_is_not_ok`,
   no mesmo lote paralelo)

   — 8/8 `done` (FECHAMENTO): `participating_if_member_fate_iff`
   (Membership.lean; um nó participa exatamente quando está no
   voter set atual), cap 40→39, floor_atom 84→85,
   floor_extract 194→193; planta DST verde
   (`participating_if_member_on_live_queued_is_not_ok`,
   no mesmo lote paralelo) — P1.1 fechado nos números exatos:
   cap 47→39, floor_atom 77→85, floor_extract 201→193 —
   status: `done`
4. **P1.2:** cadências membership 5/6 + 6/6 — `pending_joint_node`,
   `joint_target`, `joint_add_target`, `joint_leave_ok` (joint);
   `drop_repl_slot`, `drop_sent_through` (slot): ×6, cap 39→33,
   floor_atom 85→91, floor_extract 193→187 — membership ZERO
   `data_fate` medido ao vivo — status: `done`

   — 1/6 `done`: `pending_joint_node_counts_fate_iff`
   (Membership.lean; o joint pendente é definido exatamente pelos
   logs dos membros atuais), cap 39→38, floor_atom 85→86,
   floor_extract 193→192; planta DST verde
   (`pending_joint_node_counts_on_live_queued_is_not_ok`,
   6 plantas em paralelo, 1.51s)

   — 2/6 `done`: `joint_target_counts_fate_iff`
   (Membership.lean; alvo de joint-remove é o conjunto de
   membresia `ids`, exatamente quando o alvo está em `ids` —
   nunca portado pelo mapa local `nodes`), cap 38→37,
   floor_atom 86→87, floor_extract 192→191; planta DST verde
   (`joint_target_counts_on_live_queued_is_not_ok`,
   no mesmo lote paralelo)

   — 3/6 `done`: `joint_add_target_counts_fate_iff`
   (Membership.lean; alvo de joint-add é SEMPRE aceito — o
   joiner é outro pid do SO, não precisa estar no mapa local
   `nodes`; corpo constante `true`, iff com `v = true`), cap
   37→36, floor_atom 87→88, floor_extract 191→190; planta DST
   verde (`joint_add_target_counts_on_live_queued_is_not_ok`,
   no mesmo lote paralelo)

   — 4/6 `done`: `joint_leave_ok_fate_iff`
   (Membership.lean; joint commitado não é config única até um
   leave (`old == new`) estar no log — conta exatamente quando o
   leave está no log), cap 36→35, floor_atom 88→89,
   floor_extract 190→189; planta DST verde
   (`joint_leave_ok_on_live_queued_is_not_ok`,
   no mesmo lote paralelo)

   — 5/6 `done`: `drop_repl_slot_fate_iff`
   (Membership.lean; slot de replicação `next`/`match` de nó
   removido de `ids` é ESQUECIDO — corpo invertido `!in_ids`),
   cap 35→34, floor_atom 89→90, floor_extract 189→188; planta
   DST verde (`drop_repl_slot_on_live_queued_is_not_ok`,
   no mesmo lote paralelo)

   — 6/6 `done` (FECHAMENTO): `drop_sent_through_fate_iff`
   (Membership.lean; `sent_through` de nó removido via oob
   remove_member é ESQUECIDO — corpo invertido `!in_ids`), cap
   34→33, floor_atom 90→91, floor_extract 188→187; planta DST
   verde (`drop_sent_through_on_live_queued_is_not_ok`,
   no mesmo lote paralelo) — P1.2 fechado nos números exatos:
   cap 39→33, floor_atom 85→91, floor_extract 193→187 —
   membership ZERO `data_fate` MEDIDO AO VIVO no catálogo pós-
   cirurgia (bloco membership_kernel = 22/22 drenado) —
   status: `done`
5. **P1.3:** veredito datado dos medidos ausentes — SE alguma
   promoção acima medir fn/planta/handler inexistente, veredito por
   par em findings (reescrever para fn viva SE o caminho existir em
   produção; senão aposentadoria com recusa datada), espelhos/
   âncoras do 0203/0204 verdes antes/depois — status: `done`

   — veredito 2026-09-11: ZERO pares medidos ausentes nas 22
   promoções do bloco membership (22 fns + 22 as_is + 22 plantas
   DST verdes + 22 defs Lean presentes); nenhuma reescrita, nenhuma
   aposentadoria; âncoras `check_inventory_terminal` +
   `check_twin_contracts` GREEN antes (worktree `5db39a2c`,
   início da rodada) e depois (HEAD `7f733f96`)

### P2 — later / polish

6. **P2.1:** cadência final do bloco — txn ×6 (`discard_cut`,
   `leftover_txn_is_aborted`, `next_txn_id_after`,
   `prepare_error_aborts_earlier`, `reserve_si_gen`,
   `unreserve_si_gen`; wrapper `StoreTxn.lean`) + `compact_unleft`
   (wrapper `StoreCompact.lean`): ×7, cap 33→26, floor_atom
   91→98, floor_extract 187→180 — bloco cluster ZERO `data_fate`
   medido ao vivo — status: `doing`

   — 1/7 `done`: `discard_cut_fate_iff`
   (StoreTxn.lean; o corte do discard é EXATAMENTE
   `max(from, commit+1)` — nunca corta em índice cometido; o
   as-is cortava em `from` mesmo com commit acima), cap 33→32,
   floor_atom 91→92, floor_extract 187→186; planta DST verde
   (`discard_cut_on_live_queued_is_not_ok`, no lote paralelo das
   7) — wrapper `StoreTxn` inscrito no gate de extracts
   (`lean_extracts.sh` LIBS, 62 libs; buraco pré-existente desde
   o RFC-0191 fechado)
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
| P0.2 | p0 | Cadência membership 2/6 — persist+hint ×4 | done | 2b953b9a + c725658a + bc6b6f80 + este commit (4 atoms, 4 commits; números exatos) | 2026-09-11 |
| P1.1 | p1 | Cadências membership 3/6+4/6 — recover+open ×8 | done | e93c7b0f + 909dfb62 + 4b3c6ee8 + f2497120 + 96001f96 + e3f59f57 + 32b85288 + este commit (8 atoms, 8 commits; números exatos) | 2026-09-11 |
| P1.2 | p1 | Cadências membership 5/6+6/6 — joint+slot ×6; ZERO data_fate | done | fe29d5d4 + 142b2efb + 1223b824 + 38b468b4 + a8776aa8 + este commit (6 atoms, 6 commits; números exatos; membership 22/22 ZERO) | 2026-09-11 |
| P1.3 | p1 | Veredito datado dos medidos ausentes | done | este commit (veredito: 0 ausentes nas 22 promoções; âncoras green antes/depois) | 2026-09-11 |
| P2.1 | p2 | Cadência final — txn ×6 + compact ×1; cluster ZERO | doing | 1/7: este commit | 2026-09-11 |
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
