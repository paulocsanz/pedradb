# RFC: 0191 — Pacote de garantias de produto (D1/R1/T1/C1 sobre o código que corre)

**Status:** draft
**Updated:** 2026-09-10
**ID:** 0191
**Parents:** [0188](0188-segundo-degrau-close-atom.md) (circuito: close/atom/compose/∀
largos — **pago**),
[0166](0166-prova-de-fato-refinamento-propriedades.md) (enunciados D1/R1/T1/C1),
[0170](0170-refinamento-sel4-class.md) (frase permitida / frases recusadas),
[0171](0171-pagar-o-preco-sel4.md) (o termo é o corpo rustc),
[0187](0187-teorema-experimento-tcb.md) (teorema/experimento/TCB),
[0070](0070-pct-depth-not-forall-schedules.md),
[0078](0078-fsync-ok-not-media-proof.md)
**Peer:** inalterado — RocksDB default `sync=false` (`ROCKS_PARITY_SYNC=0`);
este RFC não toca parity.

> O circuito fechou. O corpus não. 276 extratos, **1 close** registado, **1
> atom** registado, **130** `if` de destino ainda no trampolim, e D1/R1/T1/C1
> ainda **modelos**. Este RFC empilha uma garantia absurda *em cima do que
> já temos*: quatro frases de produto, cada uma um teorema ∀ sem `sorry`
> sobre o fn que o rustc liga, registada, incapaz de voltar a ser modelo
> sem o ledger descer.

**A garantia absurda (o que o produto passa a poder dizer, relativo ao TCB
publicado):**

1. **R1-deleção.** Uma `Deletion` nunca é live, ∀ `range_hidden` — o get
   que passa por `visible_at` não devolve a versão apagada.
2. **D1-script.** ∀ `(need_sync, sync_fail)`: `need_sync ∧ ¬sync_fail ⇒
   AppendSyncApplyOk`; `need_sync ∧ sync_fail ⇒ Fence` (nunca Apply/Ok).
   O `commit_ops_with` de produção matchea esse plan.
3. **T1-leftover.** Todo leftover que o recover de produção classifica
   aborta; nunca materializa.
4. **C1-joint.** ∀ contagens: eleição durante joint exige maioria de
   C-old **e** C-new; C-old sozinho recusa.

Em cima disto, o trampolim só encolhe (`data_fate` cap só desce) e o
crash-injection família + exaustivo N≤3 continuam o ∀ enumerado que já
pagámos. O TCB (SO/`fdatasync`, ∀π, CRC, rustc, Z3, CPU) **permanece
recusado** — a garantia é absurda *relativa a esse TCB*, não uma prova
do disco.

Frase permitida no fim deste RFC (herdada do 0170): cada uma das quatro
frases é um `close` ou `atom` extraído do ficheiro de produção, com
corolário de produto que `unfold` essa fn. Frases recusadas: “somos
seL4”, “sem bugs”, “o fsync está provado”, “extraímos `db.rs`”.

## Background

Estado medido 2026-09-10 (pós-0188, `candidates.py` + `residuals.json` +
`close_proofs.tsv`):

| o quê | hoje | onde |
|---|---|---|
| circuito 0188 | 13/13 fatias `done` | `docs/rfc/0188-*.md` |
| proof_depth | extract=276, close registado=1, atom=1 (live close=2 via `sst_magic`) | `scripts/ratchet/proof_depth.tsv` |
| data_fate / trampolim | 130; `db_rs_extracted=false`; handler 112 092 vs kernel 23 189 LOC | `residuals.json` |
| boards 4–10 | `unpaid_script=0/17` `unpaid_compose=0/17` `unpaid_concurrency=0/5` `unpaid_scale=0/3` | `candidates.py` |
| leftover_next | **trampoline data-fate if remaining** | `candidates.py` |
| D1/R1/T1/C1 | modelos + dente (`D1Modelo.lean`, `visible_at_*`, `leftover_txn_is_aborted_true` sem ∀, `joint_election_ok_needs_both` em inputs concretos) | `formal/aeneas/lean/` |
| ∀ enumerados | crash família T∈{6,9,12} S∈{2,3,4} 33/33; exaustivo N≤3 66/66; cobertura 15/15 | ledger 0187/0188 |

O que a conversa estabeleceu e este RFC assume:

1. **O circuito não é o pacote.** Ratchet, primeiro close, primeiro atom,
   compose, três teoremas de concorrência, ∀ mais largo, voz upstream —
   isso é *como* se sobe. O pacote é *o que se afirma do banco*.
2. **Crédito 0188 vale para corolário de produto.** ∀, zero `sorry`,
   propriedade nomeada, kernel rustc, não wrap-factory, não `rfl` sobre
   inputs concretos. `wal_commit_plan_need_sync_ok` (dois bools
   concretos) e `leftover_txn_is_aborted_true` (constante) **não**
   promovem uma linha de produto. Promover exige teorema novo com
   binder `∀` que cubra o espaço da fn.
3. **Camada só sobe.** Uma garantia de produto é `model` → `atom` →
   `close`. Descer é regressão de ledger (0187 regra 2), nunca
   silencioso.
4. **Corolário unfold a fn de produção**, não o modelo. `d1_modelo`
   sozinho deixa D1 em `model`. D1 vira `close` quando o corolário
   `unfold` `wal_commit_plan` (que `commit_ops_with` matchea) ∀ sobre
   o espaço Bool da plan fn.
5. **O trampolim é o único leftover_next.** Um `if` de destino por
   commit, cap só desce, atom no mesmo commit. Montanha frozen e L28
   user-gated não entram.

## Problems This Solves

- **Problem 1 — frases de produto ainda são modelo:** D1/R1/T1/C1 do
  0166 existem; o get/commit/recover/eleição de produção não são o
  sujeito do teorema. O delete ressuscitado passou exactamente neste
  furo (invariante como teste, `if` no trampolim).
- **Problem 2 — 276 extratos ≠ 276 garantias:** sem corolário de
  produto registado, o close do heap-sift não autoriza nenhuma frase
  D1/R1/T1/C1.
- **Problem 3 — `rfl` concreto disfarça close:** teoremas sobre
  `(true, false)` ou sobre uma constante não cobrem o espaço da fn;
  um ratchet de produto sem a regra ∀ reabre a fazenda de `rfl`.
- **Problem 4 — trampolim sem meta de produto:** cap 130 impede
  crescimento mas não obriga descida; 130 `if` de destino continuam
  o sítio onde a prova não vê.
- **Problem 5 — nada impede R1-atom voltar a modelo:** o atom
  `visible_at` está no `close_proofs.tsv`; não está amarrado a uma
  linha de produto que o ledger recuse descer.

## Proposed Solution

- **A. Ratchet de produto.** TSV de 4 linhas (D1/R1/T1/C1): camada,
  `catalog:<id>`, teorema, arquivo Lean, fn rustc. Checker bloqueante:
  teorema existe, tem `∀`, zero `sorry`, catalog resolve, camada só
  sobe. `--selftest` sabota (descer camada, theorem sem ∀, dangling
  id). Job `product-floor` em `verification-gates.yml`.
- **B. Freeze honesto no dia 1.** R1 começa `atom` (já pago:
  `visible_at_deletion_never_live`). D1/T1/C1 começam `model`. Floor:
  ≥1 linha em `atom|close`. O freeze *é* a garantia absurda inicial:
  uma frase de produto já não é modelo.
- **C. Promoção ∀ do espaço da fn.** Cada P1 promove uma linha
  escrevendo o teorema `∀` que o crédito 0188 exige, `unfold` da fn
  que o handler chama, dente as-is, registro + movimento de camada no
  mesmo commit.
- **D. Trampolim com meta.** Cada `if` extraído desce `cap_data_fate`
  no mesmo commit que o atom. P2 nomeia o alvo 130→100 / `floor_atom
  ≥ 8`; a cadência continua um `if` por commit (`leftover_next`).
- **E. Inv indutivo como corolário, não como substituto.** Inv-WAL /
  Inv-LSM em P2 citam as fns close já promovidas. Não reabrem D1/R1
  como modelo.

Herdado: gate = determinístico, auto-verificante, hang=vermelho;
contagens movem no mesmo commit; `lock_interleavings_admitted` e
`forall_schedules_admitted` e `media_durable_admitted` ficam false.

## Delivery slices (mandatory)

### P0 — o pacote existe e uma frase já não é modelo

- [x] **P0.1** Ratchet de produto: `scripts/ratchet/product_guarantees.tsv`
  (4 linhas D1/R1/T1/C1) + `scripts/check_product_floor.py` + job
  `product-floor`; freeze R1=`atom` (`catalog:visible_at`,
  `visible_at_deletion_never_live`), D1/T1/C1=`model`; floor ≥1
  `atom|close`; `--selftest` 4/4 (camada desce, sem ∀, dangling,
  freeze honesto passa) — status: `done`
- [x] **P0.2** Corolário de produto R1-deleção: teorema `∀` em
  `formal/aeneas/lean/` que *nomeia* a frase R1 e `unfold`
  `merge.visible_at` (não só o atom já registado — o corolário é o
  objecto de produto); as-is dente; a linha R1 aponta para este
  teorema no mesmo commit — status: `done` (`r1_deletion_never_live`
  ∀ range_hidden, unfold `merge.visible_at`; TSV R1 aponta para ele)
- [x] **P0.3** Ledger: tabela **Garantias de produto** em
  `docs/verification-ledger.md` com as 4 frases + camada + piso
  nomeado; regra de movimento (camada só sobe no mesmo commit que o
  teorema); ponteiros `catalog:` resolvem — status: `done`

### P1 — as outras três frases e o R1 completo, sobre o espaço da fn

- [x] **P1.1** R1-value: registrar `visible_at_value_live_iff_not_hidden`
  (já é `∀ range_hidden`) + corolário de produto que unfold deletion
  **e** value; R1 permanece `atom` (não close: o get de produção ainda
  não é o sujeito — o átomo é) — status: `done` (`r1_get_atom` unfold
  ambos os braços; TSV R1 aponta para ele; atom de catálogo continua 1
  par `visible_at`)
- [x] **P1.2** D1-script `model→close`: teorema `∀ (need_sync sync_fail :
  Bool)` sobre `wal_commit_plan` (espaço inteiro da plan fn, não o par
  concreto `true false`); `commit_ops_with` continua a matchear; as-is
  Apply/Ok-antes-do-Sync; linha D1 → `close` no mesmo commit —
  status: `done` (`d1_wal_commit_plan` ∀ Bool×Bool, unfold `wal_commit_plan`
  + `fence_on_sync_fail`; TSV D1=`close` `catalog:wal_commit_plan`; close
  de catálogo continua 1 par `merge_sift` — registrar o par já extraído
  desceria `extract` abaixo do floor 276)
- [x] **P1.3** T1-leftover `model→atom`: a fn constante
  `leftover_txn_is_aborted` **não** conta; puxar o `if` de leftover do
  recover para kernel `leftover_fate(status)`; teorema `∀ status`;
  recover de produção matchea; as-is materializa; linha T1 → `atom`;
  `cap_data_fate` desce 1 se o `if` saiu do trampolim — status: `done`
  (`leftover_fate(committed)`; `t1_leftover_fate` ∀ Bool; `abort_leftover_intents`
  matchea; as-is `leftover_fate_as_is`; cap 130 intacto — o `if` era store
  recover, não trampolim `db.rs`)
- [x] **P1.4** C1-joint `model→close`: teorema `∀` sobre
  `joint_election_ok` (contagens + `Option` joint) — C-old sozinho
  recusa durante joint, ambas maiorias elegem; as-is elege com C-old;
  linha C1 → `close`; N≤3 enumerado continua o ∀ do harness (não
  substitui este close) — status: `done`
  (`c1_joint_election` ∀ contagens+Option contra a maioria pura `maj`;
  `majority_of_closed` abre div/add via `UScalar.div_bv_spec` /
  `U64.add_bv_spec`; corolários `c1_old_majority_alone_refuses` e
  `c1_both_majorities_elect`; as-is ∀ `c1_as_is_elects_on_old_alone`;
  `Membership.lean`; TSV C1=close, `floor_promoted` 4; sem linha nova em
  `close_proofs.tsv` — par já extraído, `proof_depth.extract` 276 intacto)
- [x] **P1.5** Um `if` data-fate vivo do trampolim puxado a kernel nomeado,
  atom registado, `cap_data_fate` 130→129 no mesmo commit — alvo: os `if`s
  de destino do `repair_si_hist_tip` (F52/F117; o `leftover_next` da
  fila apontava para o mesmo corpo de decisão). Kernel
  `si_hist_repair_plan(tip_gen, tip_matches) -> SiHistRepair{Leave,
  Rewrite}` em `txn_kernel.rs`; trampoline em `store/lib.rs` vira `match`
  no kernel; par `catalog:si_hist_repair` nasce atom (teorema ∀
  `si_hist_repair_plan_leave_iff_floor_or_match` em `Txn.lean`: Leave iff
  piso gen-0 ou tip já bate — `split` sobre o `ite` extraído, 0 sorry) e
  `should_repair_si_hist` gradua (perde `data_fate`). Recount honesto do
  freeze: `visible_at` (0085e919) saiu do extract sem baixar o congelado
  — `floor_extract` 276→275 e residuals `extract` 275 no mesmo commit
  (promoção de escada, não extração perdida) — status: `done`
- [ ] **P1.6** `scripts/lean_extracts.sh --required` + `check_product_floor`
  + `check_depth_floor` verdes no commit de cada promoção; nenhum
  `sorry` novo — status: `todo`

### P2 — invariante indutivo, trampolim com alvo, herdados

- [x] **P2.1** Inv-WAL (um passo): lema de preservação de
  `acked ⊆ synced ⊆ prefixo-recuperável` para `wal_append` extraído;
  corolário D1 cita este lema **e** o close P1.2; não reabre D1 como
  modelo — status: `done`
  (`WalState.lean`: `wal_inv_closed` + `wal_append_closed` +
  `wal_append_preserves_inv_wal` ∀ s s' n (desfecho ok do append
  preserva o invariant; fail/div do add checado contradizem o `ok s'`);
  corolário `d1_plan_append_preserves_inv_wal` faz `rw
  [d1_wal_commit_plan]` (close P1.2) e `exact` o lema — os 3 casos de
  plano não-SyncApplyOk fecham por contradição; linha D1 permanece
  `close`)
- [x] **P2.2** Inv-LSM (um passo): lema `visible_at` + probe-order
  newest-first (0164 já kernel) ⇒ get não devolve versão não-live;
  corolário R1 cita P0.2/P1.1 **e** este lema — status: `done`
  (`inv_lsm_newest_first_never_non_live` + corolário
  `r1_get_never_returns_non_live` em `Merge.lean`; R1 segue `atom`)
- [ ] **P2.3** Campanha trampolim com alvo nomeado: `cap_data_fate ≤
  100` e `floor_atom ≥ 8`; cadência = um `if` por commit (P1.5 é o
  primeiro); este P2.3 fecha quando o cap e o floor baterem, não numa
  sessão — status: `todo`
- [ ] **P2.4** Herdados do [0187](0187-teorema-experimento-tcb.md),
  **não re-fatiados**: P2.1 L28 user-gated, P2.2 TCG power-cut +
  `F_FULLFSYNC` nightly, P2.3 exaustivo N=4. Fronteira crash-injection
  T≤12/S≤4 permanece nomeada (alargar é movimento de ledger, não
  silêncio) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Ratchet de produto (4 linhas, camada só sobe) | done | `product_guarantees.tsv` + `check_product_floor.py` + job `product-floor` | 2026-09-10 |
| P0.2 | p0 | Corolário R1-deleção (∀, unfold `visible_at`) | done | `r1_deletion_never_live` Merge.lean | 2026-09-10 |
| P0.3 | p0 | Ledger garantias de produto | done | tabela em `verification-ledger.md` + ponteiros catalog | 2026-09-10 |
| P1.1 | p1 | R1-value (∀ `range_hidden`, ambos os braços) | done | `r1_get_atom` + `visible_at_value_live_iff_not_hidden` registado | 2026-09-10 |
| P1.2 | p1 | D1-script `model→close` (∀ Bool da plan fn) | done | `d1_wal_commit_plan` WriteAdmission.lean; TSV D1=close | 2026-09-10 |
| P1.3 | p1 | T1-leftover `model→atom` (fn deixa de ser constante) | done | `leftover_fate` + `t1_leftover_fate` Txn.lean; TSV T1=atom | 2026-09-10 |
| P1.4 | p1 | C1-joint `model→close` (∀ contagens) | done | `c1_joint_election` + `maj` Membership.lean; TSV C1=close, promoted 4 | 2026-09-10 |
| P1.5 | p1 | Um `if` do trampolim, cap 130→129 | done | `si_hist_repair_plan` + atom `si_hist_repair_plan_leave_iff_floor_or_match` Txn.lean; floor_atom 2; `should_repair_si_hist` gradua; recount extract 275 | 2026-09-10 |
| P1.6 | p1 | Gates Lean/depth/product verdes em cada promoção | todo | — | 2026-09-10 |
| P2.1 | p2 | Inv-WAL preservação (um passo) | done | `wal_append_preserves_inv_wal` + corolário `d1_plan_append_preserves_inv_wal` WalState.lean | 2026-09-10 |
| P2.2 | p2 | Inv-LSM `visible_at` ∘ probe-order (um passo) | done | `inv_lsm_newest_first_never_non_live` + corolário `r1_get_never_returns_non_live` Merge.lean (R1 segue atom) | 2026-09-10 |
| P2.3 | p2 | Alvo trampolim cap≤100 / floor_atom≥8 | doing | 20/29 caps pagos (110; …P2.3-19 `reopen_outcome` — reopen serve tudo só sem dano no WAL, F170/F171/G8; P2.3-20 `vlog_recover` — recovery recusa só com swing commitado e nenhum arquivo em disco, F51/G-swing), atoms 21/8; falta cap 110→≤100 | 2026-09-10 |
| P2.4 | p2 | Herdados 0187 (L28 / TCG / N=4) | todo | — | 2026-09-10 |

## Acceptance Criteria

- **Tests**
  - P0.1: `python3 scripts/check_product_floor.py` GREEN; `--selftest`
    4/4; descer R1 de `atom` para `model` no TSV → vermelho; theorem
    sem `∀` → vermelho; `catalog:` dangling → vermelho.
  - P0.2: `lake build` do módulo do corolário verde; statement contém
    `∀`; `unfold merge.visible_at`; zero `sorry` no ficheiro; linha R1
    do TSV aponta para este teorema no mesmo commit.
  - P0.3: `python3 scripts/check_ledger_consistency.py` GREEN; a tabela
    nova tem as 4 frases; ponteiros resolvem.
  - P1.2: o teorema D1 é `∀ (need_sync sync_fail : Bool)` (não um par
    concreto); as-is diverge em `need_sync ∧ sync_fail`; `commit_ops_with`
    ainda contém o match de `wal_commit_plan`; camada D1 = `close`.
  - P1.3: `leftover_fate` (ou nome de produção) toma o status; teorema
    `∀`; recover chama a fn; as-is materializa leftover; T1 = `atom`.
  - P1.4: `∀` sobre os argumentos de `joint_election_ok`; as-is elege
    com C-old sozinho; C1 = `close`.
  - P1.5: `cap_data_fate` no TSV de profundidade desceu 1 no mesmo
    commit que o atom; `leftover_next` não nomeia o `if` extraído.
  - P2.1/P2.2: o lema `unfold` a fn close/atom já registada; a linha
    de produto **não** desce de camada.
  - P2.3: quando fechar, `proof_depth.tsv` tem `cap_data_fate ≤ 100`
    e `floor_atom ≥ 8`; cada descida de cap veio com atom no mesmo
    commit (histórico `git log -S cap_data_fate`).
- **Telemetry / Analytics** — none — o sinal é binário (CI
  vermelho/verde); camadas e floors são TSV, não dashboard.
- **Documentation** — este RFC (status table no mesmo commit que cada
  promoção); `docs/verification-ledger.md` tabela de produto; uma
  linha em `docs/status.md`.
- **Screenshots** — none (backend/CI-only).

## Out of scope

- Provar o contrato do SO ou do disco. `media_durable_admitted` fica
  false (0078). TCG/`F_FULLFSYNC` continuam nightly 0187 P2.2.
- `forall_schedules_admitted` / `lock_interleavings_admitted` virando
  true. PCT/TSan/cobertura 15/15 continuam experimento.
- Dump de `db.rs` / `concurrent.rs`. O trampolim esvazia-se (P1.5/P2.3),
  nunca se despeja.
- Crédito de close sobre `rfl` de inputs concretos, wrap-factory
  (`a||b`, identidade, `is_empty`, `==0`), twin/cartoon.
- Re-pin Aeneas/Charon sem widen-sem-sorry. Dyn-Trait continua recusa
  medida ([aeneas#1343](https://github.com/AeneasVerif/aeneas/issues/1343)).
- Montanha frozen (`crates/montanha-fdb-recipes/**`).
- Série L28 como teorema (0187 P2.1, user-gated; campanha ≠ ∀ TCP).
- Perf / Rocks parity / sync-peer.
- Apagar `never_floor` (CPU, rustc, Verus/Z3, CRC, deps).
- “Somos seL4” / “sem bugs” / “garantia total”.
