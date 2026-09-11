# RFC: 0208 — fechar o seam store/raft: a cadência do cluster data-fate e a composição do destino

**Status:** draft
**Updated:** 2026-09-11
**Parents:** [0205](0205-degrau-composto-offlock-forall-data-fate.md)
(o degrau composto: primeiro atom do cluster store/raft
`vote_decision`, composição ∀ off-lock, fronteira datada do handler —
fechou 6/6),
[0191](0191-pacote-garantias-produto.md) (a faixa data-fate original),
[0155](0155-silent-wrong-fail-closed.md) (as recusas admitidas nunca
flipam)

Nota de régua: fatias de prova; nenhum claim de perf. Cartaz continua
sendo Pedra vs RocksDB default `sync=false` (`ROCKS_PARITY_SYNC=0`).
A escada `count`, espelhos e âncoras são do 0203/0204 (sessão
paralela) — este RFC não as toca.

> **Tese:** o 0205 pagou o primeiro data-fate do cluster store/raft
> (`vote_decision`: WouldGrant ⟺ mesmo-termo ∧ can_vote ∧
> log_up_to_date) e dois irmãos de recovery (`recover_must_apply`,
> `recover_drop_orphan_seg`) — e provou que a cadência anda (3 atoms,
> um por commit, gates GREEN em cada passo). Mas o board vivo conta
> **92** pares `data_fate` com extração e SEM teorema de destino, e
> **66 deles são UM cluster só** — o seam store/raft: 31 em
> `l28.rs`, 26 em `membership_kernel.rs`, 6 em `txn_kernel.rs`, mais
> os singletons `commit_raft` (`propose_ack_ok`),
> `grant_persist` (`grant_after_persist`) e `compact_unleft`. Cada um
> é um if cujo destino só a execução decide — a definição operacional
> de silent-wrong em potencial, concentrada no caminho que decide
> ELEIÇÃO (`grant_after_persist`), QUORUM de ack (`propose_ack_ok`),
> reconfiguração (`joint_*`, `disk_membership`, `removed_steps_down`)
> e o protocolo real TCP do L28 (31 entradas `l28_tcp_*`). Este RFC
> paga o degrau seguinte do 0205 em três movimentos: (1) fecha o
> seam RAFT inteiro — os dois singletons restantes como atoms
> registrados (`grant_persist`, `commit_raft`), deixando os TRÊS
> kernels raft (vote/commit/membership-recovery) sem nenhum
> data-fate pendente; (2) a composição do CLUSTER — recovery e
> reconfiguração como ∀ composta sobre os atoms registrados (padrão
> ponte do 0205: compor com o registrado, nunca duplicar); (3) abre
> o bloco l28 com as duas primeiras promoções e o plano datado das 29
> restantes. Alvo: `cap_data_fate` 92→84, `floor_atom` 39→47,
> `floor_extract` 239→231 — sem flipar admission nenhuma e sem claim
> de equivalência seL4.

## Background

- Escada no fechamento do 0205 (`79ea30d5`): extract 239 / close 6
  registrados (residual 7) / atom 39 / count 7 / cap_data_fate 92 /
  pairs 299 / 18 libs compose. Gates depth/product/ledger GREEN no
  worktree destacado do HEAD (`93c9434a`), sorry 0 nos wrappers
  tocados.
- Pool data_fate (board 2026-09-11): 92 pares; 66 do cluster
  store/raft. Os corpos medidos até aqui são pure-lifts tratáveis
  (`ok (commit > applied)`, `ok b`) — a lição do 0205 P1.2 aplica:
  comparação U64 no extrato é `decide` de Prop; RHS como Prop pura.
- `grant_after_persist` (vote_kernel.rs) e `propose_ack_ok`
  (commit_kernel.rs) são os únicos data_fate dos kernels raft FORA do
  membership; `vote_decision` (o terceiro) foi pago no 0205 P0.2.
  Fechando os dois, o trio raft fica inteiro no registro.
- O bloco `l28.rs` (31 entradas `l28_tcp_*_ok`) já tem dentes
  provados em `L28.lean` (propagação `ok b` por entrada, as-is sempre
  `ok true` — a mentira que a planta real TCP crava
  `l28_real_tcp_remove_member_left_on_disk`); faltam as formas ∀ de
  destino e o registro.
- Plantas DST do cluster vivem em
  `crates/pedradb-store/src/three_teeth_queued.rs`
  (`<entry>_on_live_queued_is_not_ok`) e no protocolo real
  (`l28_real_tcp.rs`) — cada promoção dirige a produção no mesmo
  commit (moldes provados do 0205: atom = cirurgia de catálogo +
  floors no MESMO commit, um por commit).

## Delivery slices (mandatory)

### P0 — must ship first (o seam raft inteiro no registro)

1. **P0.1:** atom `catalog:grant_persist` — `grant_after_persist`
   fate ∀ (voto concedido ⟺ persistido; o caminho da eleição nunca
   concede de graça); cirurgia de catálogo (del `data_fate`, add
   `atom_reason` datado), cap 92→91, `floor_atom` 39→40,
   `floor_extract` 239→238, residuals no mesmo commit; planta DST
   `grant_after_persist_on_live_queued_is_not_ok` verde.
   — status: `done` (pago como `grant_after_persist_fate_iff`
   (Vote.lean): grant ⟺ WouldGrant ∧ persist Ok, Deny e persist
   falhada nunca concedem; cap 92→91, floor_atom 39→40,
   floor_extract 239→238 no mesmo commit; planta 1/1; gates 3× GREEN)
2. **P0.2:** atom `catalog:commit_raft` — `propose_ack_ok` fate ∀
   (ack de propose ⟺ regra de commit; sem ack fantasma); cap 91→90,
   `floor_atom` 40→41, `floor_extract` 238→237, mesmo commit; planta
   `propose_ack_ok_on_live_queued_is_not_ok` verde. Ao fechar: os
   três kernels raft sem NENHUM data-fate pendente (claim datada no
   findings).
   — status: `done` (pago como `propose_ack_ok_fate_iff`
   (Commit.lean): ack ⟺ index ≤ commit_index; cap 91→90,
   floor_atom 40→41, floor_extract 238→237 no mesmo commit; planta
   1/1; gates 3× GREEN. Claim datada medida: vote_kernel.rs e
   commit_kernel.rs com ZERO data_fate pendente; membership segue
   com 26 nomeados para a cadência P1.2 — o "três kernels" do texto
   original era overclaim, o mensurável é dois fechados + recovery
   do membership já pago no 0205)

### P1 — next wave (a composição do cluster + a banda membership)

3. **P1.1:** composição do cluster em `ComposeStoreRaft.lean` (ou
   extensão do `ComposeMembershipClone.lean`): recovery e
   reconfiguração como ∀ composta sobre os ATOMS REGISTRADOS
   (`vote_decision_fate_iff`, `recover_must_apply_fate_iff`,
   `recover_drop_orphan_seg_fate_iff`, `grant_after_persist`) +
   corpos extraídos ainda não registrados — padrão ponte do 0205:
   componhe com o registrado, nunca duplica; zero sorry; twins DST
   do cluster verdes; SEM registro (motivo em findings: atravessa N
   kernels; o registro exige par único).
   — status: `done` (pago em `ComposeStoreRaft.lean` (19º compose
   lib): `election_grant_chain_fate` compõe vote×grant_persist no
   bind (grant ⟺ mesmos-termos ∧ pode-votar ∧ log-atualizado ∧
   persist Ok; Deny/persist falhada nunca concedem, via
   `vote_decision_total`) e `recovery_fate_composed` compõe o triplo
   recover_apply × recover_drop_orphan × node_counts; zero sorry;
   twins DST 7/7; motivo do não-registro em findings)
4. **P1.2:** cadência membership ×4 — `joint_leave`
   (`joint_still_active`), `disk_membership`
   (`disk_membership_overrides_cli`), `high_water`
   (`high_water_at_least`), `removed_step_down`
   (`removed_steps_down`): um atom por commit, cap 90→86,
   `floor_atom` 41→45, `floor_extract` 237→233; quedas medidas
   (corpos opacos) caem para o próximo par do cluster com recusa
   datada (padrão 0200 P1.2 / 0205 P1.2 — cadência não trava).
   — 1/4 `done`: `removed_steps_down_fate_iff` (Membership.lean;
   step-down ⟺ id saiu do conjunto committado), cap 90→89,
   floor_atom 41→42, floor_extract 237→236; planta DST 1/1

### P2 — later (o bloco l28 abre + sweep)

5. **P2.1:** primeira banda l28 ×2 — `l28_tcp_left`
   (`l28_tcp_left_ok`), `l28_tcp_hw` (`l28_tcp_hw_ok`): atoms
   registrados, cap 86→84, `floor_atom` 45→47,
   `floor_extract` 233→231; plantas reais TCP
   (`l28_real_tcp_remove_member_left_on_disk`,
   `l28_real_tcp_high_water_after_remove`) verdes; plano datado das
   29 restantes em `formal/aeneas/EXTRACT.md` (bloco l28: ordem,
   molde pure-lift `ok b` verificado nos dentes, risco de corpo
   opaco por entrada).
6. **P2.2:** sweep final: gates 3× GREEN em worktree destacado do
   HEAD final (DENTRO de `software/` — caminho relativo do backend
   Aeneas), sorry 0 nos wrappers tocados, capturas em findings,
   nota datada do seam em `EXTRACT.md` (raft fechado; cluster
   8+3/66 pagos; 55 restantes nomeados para a próxima cadência),
   flip `**Status:** done`.

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Seam raft 1/2: grant_after_persist a atom | done | grant_after_persist_fate_iff (Vote.lean) | 2026-09-11 |
| P0.2 | p0 | Seam raft 2/2: propose_ack_ok a atom (trio raft fechado) | done | propose_ack_ok_fate_iff (Commit.lean) — vote+commit kernels zero data_fate | 2026-09-11 |
| P1.1 | p1 | Composição do cluster (recovery/reconfig como ∀ sobre atoms) | done | election_grant_chain_fate + recovery_fate_composed (ComposeStoreRaft.lean) | 2026-09-11 |
| P1.2 | p1 | Cadência membership ×4 (cap 90→86) | wip (1/4: removed_step_down done) | removed_steps_down_fate_iff (Membership.lean) | 2026-09-11 |
| P2.1 | p2 | Banda l28 ×2 + plano datado do bloco (cap 86→84) | todo | — | 2026-09-11 |
| P2.2 | p2 | Sweep final + nota do seam store/raft | todo | — | 2026-09-11 |

## Critérios de aceite

1. Cada promoção: teorema ∀ de destino + linha do registro +
   floors (`floor_atom` +1, `floor_extract` −1, `cap_data_fate` −1) +
   cirurgia de catálogo + residuals + flip de status NO MESMO
   COMMIT; uma promoção por commit; planta DST do par verde
   dirigindo a produção.
2. P0 fecha o seam raft: findings datado mostrando os três kernels
   raft (vote/commit/membership-recovery) sem par `data_fate`
   pendente no catálogo.
3. P1.1 zero sorry e twins verdes; a razão de não-registro
   documentada em findings (padrão 0205 P1.1).
4. Sweep P2.2: worktree destacado DENTRO de `software/`; gates
   depth/product/ledger 3× GREEN; extracts `--required` ok; sorry 0
   nos wrappers tocados; as 3 admissions `media_durable_admitted`,
   `forall_schedules_admitted`, `lock_interleavings_admitted`
   seguem `always false`; capturas em findings.
5. Quedas medidas são recusas datadas em findings/EXTRACT.md com o
   próximo par do cluster promovido no lugar — nunca força claim.

## Out of scope

- Qualquer claim de equivalência seL4 (o proof-term cobre kernels
  que o rustc liga; o TCB nomeado no 0205 P2.1 permanece).
- Flipar `media_durable_admitted` / `forall_schedules_admitted` /
  `lock_interleavings_admitted` (recusas plantadas em produção:
  `claim_media_durable_refused_after_fsync_ok`, PCT depth 2,
  `claim_lock_interleavings_refused_after_put`).
- Extrair db.rs inteiro (112.092 LOC seguem TCB; só a nota do seam
  se move); semântica de HashMap (fronteira datada 0202 P1.1);
  ∀π sobre interleavings de ConcurrentDb.
- A escada `count`, espelhos, cotas derivadas e âncoras por classe
  (0203/0204, sessão paralela — arquivos in-flight dela nunca são
  editados aqui).
- Perf/cartaz; setup de paridade RocksDB intocado (peer oficial
  segue default `sync=false`, `ROCKS_PARITY_SYNC=0`).
- Re-pin de Charon/Aeneas/Lean sem alargamento medido sem `sorry`.
