# RFC: 0205 — o degrau composto: o commit off-lock como ∀ e o encolhimento datado do data-fate

**Status:** draft
**Updated:** 2026-09-11
**Parents:** [0202](0202-quatro-teoremas-concorrencia.md) (os quatro
teoremas de concorrência; fechou 6/6 — data-race, lost-update, deadlock
e recusa do escalonador),
[0200](0200-alcancabilidade-completa-write-path-merge.md) (passo a
passo no write path), [0191](0191-pacote-garantias-produto.md) (a faixa
data-fate original),
[0155](0155-silent-wrong-fail-closed.md) (a recusa
`lock_interleavings`)

Nota de régua: fatias de prova; nenhum claim de perf. Cartaz continua
sendo Pedra vs RocksDB default `sync=false` (`ROCKS_PARITY_SYNC=0`).
A escada `count` e suas âncoras são do 0203 (sessão paralela) — este
RFC não as toca.

> **Tese:** o 0202 pagou as quatro propriedades de concorrência no
> nível PASSO — cada kernel do `ConcurrentDb` agora tem o seu teorema
> de fatia (`rwlock_client_may_mutate`, `occ_member_fate`, ponte
> `wait_for_deadlock`; escalonador recusado com teorema de recusa em
> produção). Mas a frase seL4 do produto para no degrau seguinte, e o
> board vivo nomeia os três buracos: (1) a COMPOSIÇÃO do protocolo
> off-lock existe só como dentes CONCRETOS — `ComposeConcurrent.lean`
> prova `may_publish_group false = ok false ∧ wal_rotate_decision
> {…literal…} = ok KeepWal` em DOIS mundos pinados; nenhum ∀ registrado
> cobre "publica grupo ⟺ WAL I/O Ok" sobre todos os inputs (par
> `group_publish` no catálogo desde 0071, twin_kind close, extraído,
> chamado ao vivo por `finish_group_off_lock`, SEM linha no registro);
> (2) `cap_data_fate` segue 95 — o board conta 91 pares `data_fate`
> COM extração e SEM teorema de destino (cluster store/recovery de
> `pedra-store`: vote, joint membership, recover_apply/truncate/
> drop_orphan, persist_*), cada um um if cujo destino só a execução
> decide — a definição operacional de silent-wrong em potencial;
> (3) a camada de handler nunca foi extraída (`db_rs_extracted False`,
> `handler_loc` 112092) e nenhuma fronteira datada diz o que o
> proof-term do kernel cobre versus o que fica TCB de composição.
> Este RFC paga o degrau composto: o commit off-lock como ∀ REGISTRADO,
> a cadência datada de encolhimento do data-fate (pool 91, um por
> commit), e a fronteira do handler datada — sem flipar admission
> nenhuma e sem claim de equivalência seL4.

## Background

- Escada no fechamento do 0202 (`bacd770c`): extract 242 / close 5
  registrados (residual 6 = 5 + 1 twin sem extração) / atom 36 / count
  7 / cap_data_fate 95 / pairs 299. Gates depth/product/ledger GREEN
  no worktree destacado do HEAD.
- `may_publish_group` (group_commit_kernel.rs) é o coração
  publish-after-WAL do commit off-lock: 0071 P0 pagou kernel + twins +
  plantas (`failed_wal_sync_does_not_publish_group`,
  `may_publish_group_on_live_group_is_not_ok`); o texto do residuals
  registra a garantia — mas NÃO existe linha `close catalog:group_publish`
  no registro: o par segue no pool não-registrado, como `bearer` seguia
  antes do 0202 P1.2.
- `ComposeConcurrent.lean` (tier compose, 12 libs): conjunções
  concretas cross-lib — `concurrent_publish_and_inflight_keep_wal`
  (WAL falho não publica ∧ inflight mantém WAL),
  `concurrent_publish_ok_and_idle_rotates`, dente as-is
  (`concurrent_as_is_publish_lie_inflight_still_keeps`),
  `occ_snap_published_and_no_publish_on_wal_fail`. Duas entradas
  literais cada; o ∀ que elas instantaneiam nunca foi escrito.
- Board (2026-09-11): 91 pares `data_fate` com extração sem registro,
  concentrados no cluster store/recovery (≥25 em `pedra-store`:
  `vote_decision`, `joint_*`, `recover_*`, `persist_*`, `discard_*`,
  `force_clear`, `drop_preimages`); 145 pares não-df registráveis
  (entre eles `group_publish`). Seções A/B sem buraco: nenhum twin
  ausente, nenhum freeze hole.
- O handler (db.rs/concurrent.rs, 112k LOC) dirige os kernels
  extraídos; `db_rs_extracted False` é a borda oficial do Aeneas
  hoje — a fronteira TCB correspondente nunca foi datada em
  `formal/aeneas/EXTRACT.md`.

## Problems This Solves

- **Problem:** o cliente do produto recebe Ok de um grupo que o WAL
  ainda não confirmou? A garantia "não" está espalhada em dentes
  concretos e texto de findings — não em um ∀ registrado no mesmo
  commit do registro da escada. Quem lê o registro (`close_proofs.tsv`)
  não encontra publish-after-WAL.
- **Problem:** 91 destinos de dados sem teorema com extração pronta:
  cada if de data-fate é uma classe de bug de integridade (valor
  publicado no lugar errado, voto concedido fora da maioria, entrada
  de recovery aplicada que deveria truncar) que hoje só a campanha
  cobre.
- **Problem:** sem fronteira datada do handler, o argumento "o kernel
  que o rustc liga é o proof-term" fica sem contorno: ninguém diz onde
  ele termina (composição de handlers, escalonador, HashMap) — e a
  recusa do escalonador (0202 P2.1) fica sem endereço.

## Proposed Solution

- **P0.1 — close registrado `group_publish`:** o ∀ que os dentes
  instantaneiam — sobre o corpo extraído de `may_publish_group`
  (GroupCommitKernel.lean), na forma iff de destino:
  `may_publish_group wal_io_ok = ok wal_io_ok` para TODO input (o
  kernel é o flip perfeito: WAL Ok publica, WAL falho não publica —
  sem terceiro destino). Registro `close catalog:group_publish`,
  `floor_close` 5→6, residuals close 6→7 no mesmo commit; molde do
  quinto close (0202 P1.2: `bearer_token_from_value_fate_iff`).
- **P0.2 — primeiro atom do cluster store/recovery:** candidato
  nomeado `vote_decision` (par `vote`, store+raft, extraído em
  StoreKernel/Vote): destino do voto como iff sobre os campos de
  maioria/termo. Promoção a atom: cirurgia de catálogo (del
  `data_fate`, add `atom_reason` datado), cap 95→94,
  `floor_atom` 36→37, `floor_extract` 242→241 no mesmo commit.
- **P1.1 — a composição ∀ do off-lock (tier compose):** em
  `ComposeConcurrent.lean`, com zero sorry: (a) `∀ wal_ok,
  may_publish_group wal_ok = ok wal_ok` (corolário imediato do P0.1
  sobre o outro lib); (b) a regra de rotação ∀ sobre os campos do
  registro de pin (`KeepWal ⟺ commit_inflight ∨ parked_unflushed ∨
  pin_live…` conforme o corpo extraído decidir — a disjunção exata
  vem do extract, não da intuição); (c) os dentes concretos existentes
  viram COROLÁRIOS por instanciação. Twins DST dirigem a produção
  (`may_publish_group_on_live_group_is_not_ok` +
  `wal_segment_is_empty_on_live_zero_is_not_ok`). SEM linha de
  registro (a regra do registro exige um par/entry único; documentar
  no findings por quê o tier compose não registra — o mesmo motivo dos
  12 compose atuais).
- **P1.2 — cadência data-fate (2 promoções):** dois atoms/close do
  pool 91, um por commit, candidatos nomeados do cluster recovery
  (`recover_must_apply`, `recover_drop_orphan_seg` — destino de
  recovery como iff: aplicar exatamente quando commitado; dropar órfão
  exatamente quando fora do inventário). cap 94→93→92; quedas medidas
  (corpos `partial_fixpoint`/loops opacos) caem para o próximo par do
  board com registro datado — cadência não trava (padrão 0200 P1.2).
- **P2.1 — fronteira datada do handler:** seção datada em
  `formal/aeneas/EXTRACT.md`: o proof-term cobre os kernels que o rustc
  liga (extraídos, registrados); TCB permanece: composição de handlers
  (db.rs 112k LOC), escalonador/interleavings (recusa 0202 P2.1
  reiterada: `lock_interleavings_admitted` segue `ok false`), HashMap
  (fronteira P1.1 do 0202), e as duas outras admissions
  (`media_durable_admitted`, `forall_schedules_admitted`) nunca
  flipam.
- **P2.2 — sweep final:** gates 3× GREEN em worktree destacado do HEAD
  final, sorry 0 nos wrappers tocados, capturas em findings, flip
  `**Status:** done`.

## Delivery slices (mandatory)

### P0 — must ship first (o close composto + o primeiro data-fate)

1. **P0.1:** close registrado `catalog:group_publish`
   (`may_publish_group` iff ∀; `floor_close` 5→6, residuals close
   6→7 no mesmo commit) — molde `bearer_token_from_value_fate_iff`.
   — status: `done` (pago como `may_publish_group_ok_iff_wal_io_ok`
   (GroupCommit.lean): o corpo extraído é o lift puro
   `ok wal_io_ok`, então a forma honesta é o molde pure-lift do
   `dir_sync_required_ok_iff_sync` — publish ⟺ WAL I/O Ok, sem
   terceiro destino; molde bearer não se aplica a corpo sem bind;
   floor_close 5→6, residuals close 6→7 no mesmo commit; build
   GroupCommit verde, sorry 0; planta DST
   `may_publish_group_on_live_group_is_not_ok` 1/1)
2. **P0.2:** atom data-fate `catalog:vote` (`vote_decision`;
   cirurgia de catálogo + cap 95→94 + `floor_atom` 36→37 +
   `floor_extract` 242→241 no mesmo commit). — status: `done`
   (pago como `vote_decision_fate_iff` (Vote.lean): fate ∀ de dois
   construtores — WouldGrant ⟺ mesmo-termo ∧ can_vote ∧
   log_up_to_date, Deny na negação; totalidade derivada do
   `vote_decision_matches_spec` (o P40 grant-iff pinava só o lado
   grant); cirurgia de catálogo datada, cap 95→94, floor_atom 36→37,
   floor_extract 242→241, residuals 7/37 no mesmo commit; build Vote
   verde, sorry 0; planta DST
   `vote_decision_on_live_queued_is_not_ok` (pedradb-store) verde)

### P1 — next wave (composição ∀ + cadência)

3. **P1.1:** composição ∀ off-lock em `ComposeConcurrent.lean`
   (publish ∀ + rotate ∀ + dentes viram corolários; zero sorry; twins
   DST; SEM registro — razão documentada em findings). Se o corpo
   medir recusa, registra a recusa datada em `formal/aeneas/EXTRACT.md`
   e cai para a próxima composição tratável (off-lock occ-snap).
   — status: `done` (pago sem recusa: `concurrent_publish_fate_forall`
   (a) + `wal_rotate_decision_fate_forall`/as-is (b, disjunção exata
   vinda do corpo extraído: RotateWal ⟺ registro limpo nos 5 campos;
   KeepWal complemento; o mutante as-is droppa `pin_live` da disjunção)
   + ponte `try_rotate_step_rotates_iff_all_clear_record` compondo com
   o close registrado do Flush (registro→passo), sem duplicar; os 4
   dentes viram corolários por instanciação; zero sorry; twins DST
   `may_publish_group_on_live_group_is_not_ok` e
   `wal_segment_is_empty_on_live_zero_is_not_ok` 1/1; sem linha de
   registro — motivo documentado no findings: o tier compose atravessa
   dois kernels, o registro exige par/entry único)
4. **P1.2:** cadência data-fate: 2 promoções do pool 91 (candidatos
   `recover_must_apply`, `recover_drop_orphan_seg`; um por commit;
   quedas medidas caem para o próximo par) — cap 94→93→92.
   — status: `done` 2/2 (1/2 `recover_must_apply` pago como
   `recover_must_apply_fate_iff` (Membership.lean): re-apply
   EXATAMENTE quando commit > applied, mutante as-is pula tudo;
   cap 94→93, floor_atom 37→38, floor_extract 241→240 no mesmo
   commit; planta DST `recover_must_apply_on_live_queued_is_not_ok`
   1/1; gates 3× GREEN; 2/2 `recover_drop_orphan` pago como
   `recover_drop_orphan_seg_fate_iff` (Membership.lean): drop de
   órfão EXATAMENTE quando seg_index > new_hi, mutante as-is preserva
   todo órfão; cap 93→92, floor_atom 38→39, floor_extract 240→239 no
   mesmo commit; planta DST `recover_drop_orphan_seg_on_live_queued_
   is_not_ok` 1/1; gates 3× GREEN; uma promoção por commit, sem
   quedas medidas)

### P2 — later / fronteira + sweep

5. **P2.1:** fronteira datada do handler em `formal/aeneas/EXTRACT.md`
   (o que o proof-term cobre; TCB nomeado: handlers, escalonador,
   HashMap; admissions recusadas reiteradas).
6. **P2.2:** sweep: gates GREEN no HEAD final (worktree destacado),
   sorry 0 nos wrappers tocados, capturas, flip `**Status:** done`.

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Close composto: may_publish_group ∀ registrado | done | may_publish_group_ok_iff_wal_io_ok (GroupCommit.lean) | 2026-09-11 |
| P0.2 | p0 | Primeiro data-fate do cluster store: vote_decision a atom | done | vote_decision_fate_iff (Vote.lean) | 2026-09-11 |
| P1.1 | p1 | Composição ∀ off-lock (publish × rotate; dentes viram corolários) | done | concurrent_publish_fate_forall + wal_rotate_decision_fate_forall + ponte try_rotate_step_all_clear (ComposeConcurrent.lean) | 2026-09-11 |
| P1.2 | p1 | Cadência data-fate: +2 promoções (cap 94→92) | done | recover_must_apply_fate_iff + recover_drop_orphan_seg_fate_iff (Membership.lean) | 2026-09-11 |
| P2.1 | p2 | Fronteira datada do handler (EXTRACT.md) | todo | — | 2026-09-11 |
| P2.2 | p2 | Sweep final de gates | todo | — | 2026-09-11 |

## Acceptance Criteria

- **Tests:** cada atom/close roda `scripts/lean_extracts.sh --required`
  verde; gates `depth-floor` + `product-floor` + `ledger` GREEN no
  commit de cada promoção; registro (TSV/floors/residuals/catalog)
  no MESMO commit do teorema (regra do ratchet); planta DST nomeada
  do par verde dirigindo a fn de produção.
- **Teoremas:** enunciado com `∀` literal (exigência mecânica do
  gate de profundidade — binders diretos não passam); zero `sorry`
  no arquivo; hipóteses pontuais para chamadas-axioma (core.str,
  HashMap) no molde das pontes do 0202 — o que for opaco fica opaco e
  datado, nunca simulado.
- **Cadência:** uma promoção por commit; queda medida (corpo
  `partial_fixpoint`, loop opaco) documentada com data e razão antes
  de cair para o próximo par — cadência não trava.
- **Fronteira (P2.1):** a seção do EXTRACT.md nomeia o TCB por
  categoria (handlers/escalonador/HashMap) e cita as três admissions
  recusadas por nome; nada é flipado em `scripts/formal/
  residuals.json`.

## Out of scope

- Claim de equivalência seL4 (a régua do repo segue: mais perto o
  possível, nunca "somos seL4").
- Flips das três admissions recusadas: `media_durable_admitted`,
  `forall_schedules_admitted`, `lock_interleavings_admitted` (seguem
  `ok false`; a recusa do escalonador tem teorema de recusa em
  produção — `claim_lock_interleavings_refused_after_put`).
- ∀π sobre interleavings do ConcurrentDb (TCB nomeado na fronteira
  P2.1, coberto por campanha PCT/TSan).
- A escada `count`, âncoras de classe de host e cotas por máquina
  (RFC-0203, sessão paralela — arquivos in-flight não são editados
  aqui; twins de contagem seguem o contrato de lá).
- Semântica de HashMap/HashSet no Aeneas (fronteira datada do 0202
  P1.1 segue de pé).
- Claims de perf/cartaz; setup de paridade RocksDB intocado (peer
  oficial default `sync=false`, `ROCKS_PARITY_SYNC=0`).
- Re-pin de Charon/Aeneas/Lean sem alargamento medido sem `sorry`.
