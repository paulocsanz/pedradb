# RFC: 0188 — Aprofundamento drástico: o segundo degrau (close, atom, composição, ∀ mais largos)

**Status:** draft
**Updated:** 2026-09-10
**ID:** 0188
**Parents:** [0187](0187-teorema-experimento-tcb.md) (teorema/experimento/TCB + circuitos
de gate determinísticos),
[0166](0166-prova-de-fato-refinamento-propriedades.md) (corpus pinado por sha256),
[0151](0151-three-teeth-as-is-verus-dst.md) (contrato three-teeth),
[0171](0171-pagar-o-preco-sel4.md) (o preço é o corpo rustc, não um twin),
[0170](0170-refinamento-sel4-class.md),
[0070](0070-pct-depth-not-forall-schedules.md) (campanha ≠ ∀)
**Peer:** inalterado — RocksDB default `sync=false` (`ROCKS_PARITY_SYNC=0`);
este RFC não toca parity.

> A formalização do Pedra tem base sem altura: **276 corpos extraídos para
> Lean, 0 provas `close`, 0 átomos `atom`** (`residuals.json` proof_depth).
> O circuito de garantias do 0187 (gates exaustivos, ratchets, ledger)
> está de pé; o que não existe é a subida. Este RFC paga o segundo degrau
> da escada — e cada degrau ganha ratchet para nunca mais descer: primeiro
> `close`, primeiro `atom`, composição dual-unfold de verdade, os quatro
> teoremas de concorrência em série, e o alargamento dos ∀ exaustivos.

## Background

Estado medido (dono no repo, capturado nesta conversa):

| o quê | hoje | onde |
|---|---|---|
| proof_depth | **extract=276, close=0, atom=0**, model=17 | `scripts/formal/residuals.json` |
| catálogo | 294 pares (proof=261, campaign=33); single_artifact=287; aeneas_scripts=227; **data_fate=130** | `scripts/formal/catalog.json` |
| trampolim | `db.rs`/`concurrent.rs` não-extraídos por decisão (`db_rs_extracted=false`); kernel_loc 26.553 vs handler_loc 109.964 | `residuals.json` glue |
| gates determinísticos | exaustivo 66/181 nós; ratchet 15 seeds; crash-injection 9 pontos (**1 workload**); barreiras 145 sítios/59 entradas; cobertura **11/15**; ledger consistente | RFC-0187 P0/P1 (`verification-gates.yml`) |
| upstream | Aeneas origin/main +89 commits **não alarga**; bloqueio raiz dyn-Trait **no nível do tipo** (`Box<dyn Iterator>` em `StreamingVisibleIter`); charon whole-crate OK (llbc 8,8 MB exit 0) | fire 803, `findings/2026-09-09-aeneas-iterator-widen.md` |
| concorrência | quatro teoremas mapeados (data-race client, lost-update, deadlock, scheduler-∀π-recusado), **nenhum pago**; compose dual-unfold vazio; única forma fechada é `lock_interleavings_admitted=false` | `findings/2026-09-07-concurrency-proof-map/` |

O que esta conversa estabeleceu e o RFC assume como axiomas de método:

1. **A escada é extract → close → atom.** Extrato só prova que o corpo
   traduziu. `close` = teorema ∀ sem `sorry` sobre um kernel de produção
   matriculado, com propriedade nomeada (não igualdade definicional).
   `atom` = `close` no átomo que decide o destino de um dado. Os degraus
   2 e 3 estão em zero — é isso que "aprofundar drasticamente" significa.
2. **Composição é o degrau escondido.** Unfold só do callee não paga o
   caller; `native_decide` sem `unfold` não é compose; `WriteAckLedger`
   reporta, não escolhe. O trampolim se esvazia com **plan fn viva**
   (o handler de produção matchea) + dual-unfold caller/callee — nunca
   com dump de `db.rs`/`concurrent.rs`.
3. **∀ exaustivo é redimensionável por construção.** O crash-injection
   mede a contagem no MESMO seam que injeta (zero drift); alargar de 1
   workload para uma família (T,S) é o mesmo truque num grid. Campanha
   estatística continua hunt/nightly — nunca gate.
4. **A rota dos iteradores é Isolated-method, não re-pin** (bloqueio
   medido no nível do tipo; regra permanente re-pin-só-se-widen-sem-
   `sorry`, 0187 P1.4).
5. **Crédito é agnóstico de ferramenta, não de corpo.** Lean (Aeneas),
   Verus ou Kani valem **somente** sobre o corpo rustc linkado e os tipos
   que o handler passa (RFC-0151/0171). Twin/cartoon não vale. Kernel
   wrap-factory (`a||b`, identidade, `is_empty`, `==0`) **não gera
   crédito de close** — senão o ratchet vira fazenda de `rfl`.

## Problems This Solves

- **Problem 1 — degrau sem prova:** 276 extratos, 0 closes, 0 atoms. A
  formalização afirma "traduziu", nunca "vale".
- **Problem 2 — ganho sem ratchet:** nada impede o primeiro close virar
  zero de novo (prova apagada, sorry infiltrado, extração regredida).
  Nenhum floor versiona profundidade.
- **Problem 3 — composição não paga:** o board `unpaid_compose` segue
  vazio; plan fns existem como **modelo** (`d1_modelo_kernel::put_ok`,
  `WriteAckLedger`) mas o script vivo (`commit_ops_with`) não matchea
  nenhuma.
- **Problem 4 — concorrência mapeada, não provada:** lost-update,
  data-race client e deadlock têm kernel e mapa mas nenhum teorema
  terminal; N-way só existe em exemplo.
- **Problem 5 — ∀ estreito:** crash-injection cobre UM workload fixo;
  4 de 15 sítios de seam (`E.create_open`, `E.remove`, `E.meta`,
  `W.crash`) não têm seed pinada.
- **Problem 6 — upstream sem voz:** a negativa dyn-Trait está medida e
  fichada, mas não existe issue/PR upstream com repro mínima nem série
  de conversão Isolated com fim declarado.

## Proposed Solution

- **A. Ratchets de profundidade:** TSV versionado com floors
  extract/close/atom + cap de `data_fate`; job bloqueante lê o TSV e o
  registro vivo; vermelho em encolhimento (ou crescimento de data_fate).
  Ganho que não virou floor é ganho não-registrado.
- **B. Primeiros degraus com regra de crédito:** registro de provas
  (`close_proofs.tsv`: id, `catalog:<id>` do kernel, teorema, arquivo
  Lean) cross-checkado mecanicamente (teorema existe, sem `sorry`, kernel
  fora da lista wrap-factory, propriedade ≠ igualdade definicional).
- **C. Série de composição:** script G1 ao vivo (plan fn que
  `commit_ops_with` matchea; teorema `need_sync ⇒ Sync antes de
  Apply/Ok`, contrafactual as-is) + primeiro dual-unfold caller/callee.
- **D. Série de concorrência:** os três teoremas pagáveis em ordem
  (lost-update N-way → data-race client → deadlock); o quarto
  (scheduler ∀π) **permanece na forma recusada**
  (`lock_interleavings_admitted=false`) — virar true está fora de scope.
- **E. ∀ mais largos:** crash-injection em família (grid T×S) e
  cobertura 15/15; cada alargamento move a linha no ledger
  (`docs/verification-ledger.md`) no mesmo commit, pela regra de
  movimento (experimento → teorema só com gate de destino verde).
- **F. Upstream:** completar a série Isolated-method até `EXTRACT.md`
  não ter recusa Iterator sem negativa medida; abrir voz upstream
  (repro mínima dyn-Trait) mantendo a regra de re-pin.

Regra transversal herdada do 0187: **gate = determinístico,
reproduzível, auto-verificante** (asserção de contagem exata + selftest
de sabotagem + hang=vermelho); o não-determinístico é nightly.
Contagens de catálogo/residuals/ledger movem no MESMO commit que o
código que as move.

## Delivery slices (mandatory)

### P0 — degraus com ratchet (a base ganha piso)

- [x] **P0.1** Ratchet de profundidade: `scripts/ratchet/proof_depth.tsv`
  (floors extract/close/atom, cap data_fate) +
  `scripts/check_depth_floor.py` + job bloqueante `depth-floor` em
  `.github/workflows/verification-gates.yml`; cross-checka TSV ==
  `residuals.json` == registro vivo; `--selftest` sabota dos dois lados
  (floor encolhido, residual stale) — status: `done`
  (floors extract=276/close=0/atom=0, cap data_fate=130, série
  handler_loc=109964; selftest 4/4: floor encolhido, residual stale,
  data_fate crescido, registro quebrado)
- [x] **P0.2** Primeiro `close` (0→1): teorema ∀ sem `sorry` sobre
  kernel de produção matriculado, propriedade nomeada; candidatos
  medidos: `sift_step` do heap-sift (0187 P1.3 — "sob ordem total
  consistente, o mínimo de filhos+buraco vai para o topo") ou
  `WriteAckLedger::d1_holds_every_cut`; registro `close_proofs.tsv` +
  `proof_depth.close=1` + floor P0.1 no MESMO commit; dente as_is
  quebra a propriedade — status: `done`
  (kernel `merge_sift` (`sift_step`/`sift_step_as_is`, Isolated-method —
  produção tem a ORDEM, kernel tem a ESTRUTURA); teoremas
  `merge_sift_step_repairs_iff` (Stay ⟸ nenhum reparo, registrado como
  primeiro close), `merge_sift_step_swap_right_iff`,
  `merge_sift_step_as_is_diverges_on_repair`; dente 3 on-live:
  `merge_heap_sift_kernel_three_teeth` — 4 streams em ordem DESC do head
  obrigam heapify a reparar; as-is emitiria `b35` antes de `b05`;
  `proof_depth.close=1` + floor + registro + catálogo (295) no mesmo
  commit)
- [x] **P0.3** Cap do trampolim: `data_fate ≤ 130` versionado (linha do
  TSV de P0.1); vermelho se um átomo data-fate novo entrar sem sair
  outro; série `handler_loc` registrada por commit (série, não piso);
  redução continua por fire — o gate garante monotonicidade — status:
  `done` (`cap_data_fate 130` + `handler_loc 109964` em
  `scripts/ratchet/proof_depth.tsv`; selftest dente data-fate-growth)

### P1 — composição paga (script, glue, concorrência)

- [x] **P1.1** Script G1 ao vivo: plan fn total que o
  `commit_ops_with` de produção matchea (append → sync se `need_sync` →
  apply/publish → Ok; fence se sync falhou); teorema
  `need_sync ⇒ Sync antes de Apply/Ok` + contrafactual as-is
  (Apply/Ok antes do Sync); `WriteAckLedger` segue report, não choose —
  status: `done` (`wal_commit_plan` matcheado em 5 sítios de produção
  (db.rs:2057/2102/3695/6568/8789 + dentro de `commit_ops_with`
  db.rs:9158/9167); `wal_commit_plan_need_sync_ok` +
  `wal_commit_plan_fence_via_fence_on_sync_fail` (dual-unfold com
  `fence_on_sync_fail`) + `wal_commit_plan_as_is_dente` em
  `WriteAdmission.lean`; planta on-live `wal_commit_plan_on_live_sync_fail_is_not_ok`
  verde; pago por fires da campanha grind, verificado 2026-09-10)
- [x] **P1.2** Primeiro compose dual-unfold: Lean `unfold` do plan fn
  QUE o handler chama (`occ_batch_plan`/`wal_commit_plan`) E do callee
  (`group_validate`/`occ_conflict`) num input representativo + ramo
  as-is; `native_decide` sem unfold não conta — status: `done`
  (`occ_batch_plan_lagging_conflict` desdobra plan + `occ_member_fate` +
  `occ_conflict` (GroupCommit.lean); caller de produção
  `validate_occ_batch` chama `occ_batch_plan` (concurrent.rs:1718/1771);
  as-is `occ_batch_plan_as_is_dente`)
- [x] **P1.3** Concorrência 1 — lost-update N-way: `group_validate` de
  N>2 `OccRead`s sobre um `last_seq`; teorema serializável vs as-is
  serialized; planta DST nomeia o kernel — status: `done`
  (`occ_batch_plan_n3_one_lagging`: 3 membros, um `last_seq`, membro
  lagging → Conflict (serializável), as-is → Ok; `group_validate`
  desdobrado em `group_validate_lagging_member_conflicts`; planta
  `occ_batch_plan_on_live_lagging_is_not_ok` verde; catálogo
  `occ_batch_plan`/`group_validate` com dst_plant)
- [x] **P1.4** Concorrência 2 — data-race (write-lock client):
  `wal_rotate_decision` + `commit_inflight` mantém o WAL no idle
  rotate; método de tokens da literatura (VerusSync / CapybaraKV
  OSDI'25) fichado em `findings/` e aplicado — status: `done`
  (`WalPinState.commit_inflight` → `KeepWal` (flush_kernel.rs);
  as-is `wal_rotate_decision_as_is_ignore_pin`; teoremas em
  `Flush.lean`; planta `wal_inv_on_live_recording_is_not_ok` verde;
  token method aplicado: `rwlock_client_may_mutate_needs_write` +
  `occ_snap_lock_order`; fichamento
  `findings/2026-09-07-capybarakv-unverified-crate`)
- [x] **P1.5** Concorrência 3 — deadlock: `wait_for_deadlock` sem
  ciclo no lock-order do ConcurrentDb (inflight vs flush rotate);
  as-is admite o ciclo — status: `done` (`wait_for_deadlock` +
  as-is em `rocksdb-compat/src/locktab.rs`; `Locktab.lean`
  `wait_for_deadlock_is_loop` + ciclo 2/3 nós; lock-order client
  `occ_snap_lock_order` (read_held/inflight); 4 testes locktab verdes
  incl. `wait_for_deadlock_on_live_cycle_is_not_ok`;
  `lock_interleavings_admitted` segue false — assert vivo
  concurrent.rs:4568)
- [x] **P1.6** Primeiro `atom` (0→1): `close` sobre o átomo data-fate
  de um handler vivo (o `if` que decide destino do dado já roteado por
  kernel); `proof_depth.atom=1` + registro + floor no MESMO commit —
  status: `done` (atom: `visible_at_deletion_never_live` ∀ range_hidden,
  Merge.lean — deleção nunca surfaced live; par `catalog:visible_at`
  com `atom_reason`; registro kind=atom em `close_proofs.tsv`;
  `floor_atom 1` + residuals atom=1 no mesmo commit)

### P2 — ∀ mais largos e upstream

- [x] **P2.1** Crash-injection em família: grid (T,S) até limites
  medidos, cada workload com contagem medida no mesmo seam, asserção de
  contagens e oracle fail-closed; ∀ sobre a família com fronteira
  nomeada (o que ficou fora do grid); `--selftest` — status: `done`
  (família de 4 workloads: T∈{6,9,12} S∈{2,3,4}; 33/33 crash points;
  `--selftest` 9/9; fronteira max T=12 max S=4 no ledger)
- [x] **P2.2** Cobertura 15/15: seeds pinadas cobrindo `E.create_open`,
  `E.remove`, `E.meta`, `W.crash` via soak adaptativo; fold no
  `coverage_floor.tsv` (P1.1/0187); remoção de seed → vermelho —
  status: `done` (3 seeds irredundantes 0x1/0x15/0xc8, union_pop=15,
  `--selftest` S4 3/3 load-bearing; widen opt-in `buggify_widen_sites`)
- [x] **P2.3** Isolated-method completo: toda recusa Iterator do
  `EXTRACT.md` convertida (heap-sift 0187 P1.3 é a primeira) OU com
  negativa medida por item nomeada no `EXTRACT.md`; fim declarado da
  série — status: `done` (tabela "Refused — Iterator / dyn shapes" no
  `EXTRACT.md`: 6 sítios, cada um com negativa RE-MEDIDA no pin em
  2026-09-10 — `Returns inside of nested loops`, `Breaks to outer
  loops`, `Could not match the contexts`, `Dynamic trait types…` —
  série declarada encerrada na seção)
- [x] **P2.4** Voz upstream dyn-Trait: issue/PR no Aeneas com repro
  mínima (`Box<dyn Iterator>` → "Dynamic trait types are not supported
  yet"); link no `docs/upstream-watch-protocol.md`; re-pin continua
  só-com-widen-sem-sorry — status: `done` (repro in-repo
  `formal/aeneas/repro/dyn-iterator/run.sh`: controle extrai, dyn
  recusa, exit 0 = watch armado; issue
  [aeneas#1343](https://github.com/AeneasVerif/aeneas/issues/1343)
  aberta 2026-09-10 com a repro e a pergunta de tracking; watch
  protocol atualizado com o procedimento de re-teste)

Herdados do [0187](0187-teorema-experimento-tcb.md) — permanecem lá,
não re-fatiados aqui: P1.3 heap-sift (em andamento), P2.1 série L28
(user-gated), P2.2 TCG power-cut + `F_FULLFSYNC` nightly, P2.3
exaustivo N=4.

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Ratchet de profundidade (floors + cap data_fate) | done | `check_depth_floor.py` + `proof_depth.tsv` + job `depth-floor` | 2026-09-10 |
| P0.2 | p0 | Primeiro `close` (0→1) com regra de crédito | done | `merge_sift` + `merge_sift_step_repairs_iff` + `close_proofs.tsv` | 2026-09-10 |
| P0.3 | p0 | Cap do trampolim (data_fate ≤ 130 monotônico) | done | `cap_data_fate`/`handler_loc` em `proof_depth.tsv` | 2026-09-10 |
| P1.1 | p1 | Script G1 ao vivo (plan fn + teorema de ordem) | done | `wal_commit_plan` + `WriteAdmission.lean` + planta on-live | 2026-09-10 |
| P1.2 | p1 | Primeiro compose dual-unfold (caller + callee) | done | `occ_batch_plan_lagging_conflict` (GroupCommit.lean) | 2026-09-10 |
| P1.3 | p1 | Lost-update N-way (`group_validate`) | done | `occ_batch_plan_n3_one_lagging` + planta on-live | 2026-09-10 |
| P1.4 | p1 | Data-race write-lock client (`wal_rotate_decision`) | done | `WalPinState.commit_inflight` + `Flush.lean` + capybarakv finding | 2026-09-10 |
| P1.5 | p1 | Deadlock (`wait_for_deadlock` sem ciclo) | done | `locktab.rs` + `Locktab.lean` + 4 testes | 2026-09-10 |
| P1.6 | p1 | Primeiro `atom` (0→1) em handler vivo | done | `visible_at` atom ∀ + registro + `floor_atom 1` | 2026-09-10 |
| P2.1 | p2 | Crash-injection em família (grid T×S) | done | 4 workloads T∈{6,9,12} S∈{2,3,4}; 33/33; selftest 9/9 | 2026-09-10 |
| P2.2 | p2 | Cobertura 15/15 (4 sítios do soak) | done | floor_pop 15; seeds 0x1/0x15/0xc8; S4 3/3 load-bearing | 2026-09-10 |
| P2.3 | p2 | Isolated-method completo (fim da série) | done | tabela de negativas re-medidas no `EXTRACT.md` (6 sítios) | 2026-09-10 |
| P2.4 | p2 | Voz upstream dyn-Trait (repro mínima) | done | repro `dyn-iterator/run.sh` + [aeneas#1343](https://github.com/AeneasVerif/aeneas/issues/1343) | 2026-09-10 |

## Acceptance Criteria

- **Tests**
  - P0.1: floor encolhido no TSV ou residual divergente do registro →
    vermelho; `--selftest` prova ambos os lados da sabotagem.
  - P0.2: teorema sem `sorry`; o mutante as_is quebra a propriedade
    (dente three-teeth); `scripts/lean_extracts.sh --required` verde;
    `residuals.json` + `close_proofs.tsv` movem no mesmo commit.
  - P0.3: novo átomo data_fate sem remoção equivalente → vermelho.
  - P1.1: teste de produção chama o handler real e o plan fn no mesmo
    caminho (`commit_ops_with` matchea); teorema + contrafactual as-is.
  - P1.2: Lean `unfold` do plan E do callee no mesmo teorema; proof sem
    unfold não é aceito como compose.
  - P1.3–P1.5: teorema e planta DST nomeiam o kernel de produção; as-is
    diverge no contraexemplo de cada propriedade.
  - P1.6: `proof_depth.atom` move junto com registro e floor.
  - P2.1: contagem por workload asserida dentro do runner; fronteira do
    grid (T,S máximos) nomeada no ledger.
  - P2.2: `coverage_floor.tsv` em 15/15; remoção de seed → vermelho.
  - P2.3: `EXTRACT.md` sem recusa Iterator que não tenha negativa
    medida ao lado.
  - P2.4: link da issue/PR no watch protocol; pins continuam congelados.
- **Telemetry / Analytics** — none — o sinal é binário (CI
  vermelho/verde) e floors/contagens são versionados e asseridos dentro
  do runner; profundidade vira TSV, não dashboard.
- **Documentation** — este RFC; `docs/verification-ledger.md` movido no
  mesmo commit de cada alargamento de ∀ (regra de movimento);
  `EXTRACT.md`/`docs/upstream-watch-protocol.md` para P2.3/P2.4.
- **Screenshots** — none (backend/CI-only).

## Out of scope

- Provar o contrato do SO ou do disco; rigs físicos de power-cut (TCB
  nomeado no ledger; TCG/`F_FULLFSYNC` seguem como nightly 0187 P2.2).
- `media_durable_admitted` / `forall_schedules_admitted` virando true —
  ficam false por decisão registrada (RFC-0078, RFC-0070).
- Dump de `db.rs`/`concurrent.rs` — o trampolim se esvazia (plan fn +
  kernel + dual-unfold), nunca se despeja.
- Mint de twin/cartoon; wrap-factory; crédito de close sobre kernel de
  identidade — a regra de crédito do P0.2 vale para toda a série.
- Re-pin de Aeneas/Charon sem widen medido sem `sorry` (regra
  permanente, 0187 P1.4).
- Perf: qualquer refactor de produção que possa mover números (ex.:
  trocar `Box<dyn Iterator>` por enum dispatch) sai deste RFC e passa
  pelo circuito otimizar com medição Linux (cartaz), nunca pela série
  formal.
- Montanha frozen (`crates/montanha-fdb-recipes/**`) e demais
  user-gated sem decisão explícita (série L28 incluso — 0187 P2.1).
- Comparações com sync-peer ou lead tables com a coluna sync (regras
  Rocks parity do repo permanecem).
