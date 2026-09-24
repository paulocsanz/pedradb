# RFC: 0227 — Prova de produto: D1/R1/T1/C1 sobre put/get/recover/replica, um `pedra_refines`, métrica honesta

**Status:** done
**Updated:** 2026-09-15
**ID:** 0227
**Parents:** [0166](0166-prova-de-fato-refinamento-propriedades.md)
(enunciados-alvo D1/R1/T1/C1),
[0191](0191-pacote-garantias-produto.md) (piso de produto sobre átomos/planos — **pago e insuficiente**),
[0198](0198-composicao-registrada-invariantes-indutivos.md) (Inv um-passo / cadeias de modelo),
[0200](0200-alcancabilidade-completa-write-path-merge.md) (alcançabilidade no modelo WAL/LSM),
[0202](0202-quatro-teoremas-concorrencia.md) (kernels de grupo, não o `ConcurrentDb`),
[0220](0220-escada-de-composicao-encadeia-os-atomos-dual-unfold.md)
(espinha do writer; P0.1 gate ainda `todo`; m2 gamed),
[0222](0222-escada-ate-o-par-sel4-gap-por-eixo-com-denominador.md) /
[0224](0224-depois-do-piso-verde-espinha-e-os-quatro-terminais.md) /
[0225](0225-depois-da-espinha-as-espinhas-0220-e-o-denominador-a2a.md)
(DEFINING 100% por nome de teorema e rename `*_kernel.rs`),
[0171](0171-pagar-o-preco-sel4.md) (o termo é o corpo rustc),
[0187](0187-teorema-experimento-tcb.md) (teorema / experimento / TCB),
[0188](0188-segundo-degrau-close-atom.md) (crédito ∀, wrap-factory ban),
[0061](0061-residuals-sel4-ironfleet.md) (classe de claim, não de garantia)
**Peer:** inalterado — RocksDB default `sync=false` (`ROCKS_PARITY_SYNC=0`).
Este RFC não toca parity.

> O corpus prova que cada kernel extraído faz o que o próprio corpo diz.
> Não prova que o banco, depois de um put acked, um get, um recover de TX
> ou um read numa réplica, satisfaz D1/R1/T1/C1. Este RFC paga esse
> recuo: subir R1/T1 de atom→close **sobre o get/recover**, D1 sobre
> put→crash→reopen, C1 sobre o valor servido, e **um** teorema de
> refinamento que encadeia WAL+LSM+OCC contra a spec. Sem mais
> `fate_iff` como crédito de produto. Sem mais
> `def m2_fn_* : String`.

**Frase permitida no fim deste RFC** (relativa ao TCB publicado, nunca
absoluta): *D1, R1, T1 e C1 valem do put/get/recover/replica-read que o
rustc liga, via `pedra_refines`, zero `sorry`.*

**Frases recusadas:** “somos seL4”, “sem bugs”, “o fsync está
provado”, “extraímos `db.rs`”, “DEFINING 100%”.

## Background

Estado medido 2026-09-15 (`candidates.py`, `sel4_gap.py`,
`product_guarantees.tsv`, `close_proofs.tsv`, `ComposeDefining.lean`,
`ComposeM2.lean`):

| o quê | hoje | o que isso **não** é |
|---|---|---|
| escada 0188 | 330/330 pares; extract 0; **6 close** / **326 atom** / 7 count | 326 iff locais ≠ 4 frases de produto |
| produto 0191 | D1=`close` `wal_commit_plan`; R1=`atom` `visible_at`; T1=`atom` `leftover_fate`; C1=`close` `joint_election_ok` | D1 não é put-crash-reopen; R1 não é o get; T1 não é o recover; C1 não é o valor servido |
| DEFINING | print **100%** | A1 = alias de 3 bools nomeado `pedra_refines`; A3 = `visible_at × tombstone` nomeado `confinement`; A6 = `occ_conflict × occ_member_fate` nomeado `concurrent_db_write_group_forall`; A2a = todo src `*_kernel.rs` (incl. trampolim) |
| composição m2 | 324/326 = 99,39% | `ComposeM2.lean`: **270** `def m2_fn_X : String := "X"` e **um** `theorem` (`rfl` do nome). Os 2 de fora são `seal_async_first_drain` / `solo_leader_bypass` (RFC-0226) |
| trampolim | `cap_data_fate=0`; `unpaid_trampoline_data_fate_ifs=2` | `concurrent_kernel.rs::lead`: `if self.solo_bypass` e `if pending.ops.len() == 1` |
| get/scan | `visible_at` / `iter_window_keep` por item | `StreamingVisibleIter` / `WindowKvIter` (`Box<dyn Iterator>`) recusados no tipo; o walk que o rustc liga **não** é o termo Lean |
| Inv | `wal_write_step_reach` / `merge_chain` | indução sobre o **modelo** (contadores WAL, cadeia de merges), não sobre o estado alcançável do engine |
| L28 | 33 pares campaign | planta, não a relação “valor servido ∈ prefixo majority-durable” |
| board 4–10 | script/compose/concurrency/scale = 0 unpaid | lista fixa de glue; não cobre as espinhas 0220 P1 nem o sujeito get/recover |

O 0191 fez o que prometeu: quatro linhas de produto amarradas a fns
rustc, camada só sobe. O sujeito de cada linha **não** é o enunciado
0166. `r1_get_never_returns_non_live` ainda é
`visible_at kind range_hidden = ok true → kind = Value ∧ ¬hidden`.

Os 326 `*_fate_iff` ficam — são os **passos** da simulação. Este RFC
proíbe usá-los como crédito de *produto* e proíbe mintar mais um para
subir camada. Wrap-factory (`batch_is_empty`, `dir_sync_required`,
`a\|\|b`, `==0`) nunca move D1/R1/T1/C1.

Rota dos iteradores: Isolated-method no ficheiro que o rustc liga
(mesmo preço do `probe_order_covering` / `sift_step`). Não esperar
dyn-Trait ([aeneas#1343](https://github.com/AeneasVerif/aeneas/issues/1343)).
Não re-pin sem widen-sem-`sorry`.

Este RFC **absorve** [0220](0220-escada-de-composicao-encadeia-os-atomos-dual-unfold.md)
P0.1 e [0225](0225-depois-da-espinha-as-espinhas-0220-e-o-denominador-a2a.md)
P0.1 (gate de composição ainda inexistente: `scripts/check_compose_floor.py`
não está no disco). O gate nasce aqui, com a definição honesta. As
espinhas 0220 P1 que ainda são `todo` migram para P2.4.

[0187](0187-teorema-experimento-tcb.md) P2.1 (L28 user-gated) é
**levantado só para a frase C1 deste RFC** — um teorema de refinamento
sobre os kernels de log/commit/apply/joint, não promoção das 33 plantas
TCP a ∀.

## Problems This Solves

- **Problem 1 — sujeito errado.** D1/R1/T1/C1 estão “pagos” sobre o
  predicado que o handler chama, não sobre put/get/recover/replica.
- **Problem 2 — sem refinamento de sistema.** Não há um teorema “a
  implementação refina o dicionário-com-crash”. `pedra_refines` hoje é
  um alias.
- **Problem 3 — métrica gamed.** m2 conta substring em `Compose*.lean`.
  DEFINING conta o nome do teorema e o glob `*_kernel.rs`. O board
  imprime 100% com os quatro furos intactos.
- **Problem 4 — composição contabilizada, não encadeada.** Writer-spine
  e recovery-spine existem. Auto-flush, OCC N-way do handler,
  parked_pop, changelog, lock-order×rotate, ciclo 2PL não.
- **Problem 5 — o termo não é o walk.** Get/scan de produção passam por
  `dyn Iterator`; o Lean vê o filtro por item.
- **Problem 6 — glue ainda escolhe.** Dois `if`s em `lead`; handler
  matcheia o plano e faz o Env — a ordem I/O do handler não é termo.
- **Problem 7 — ConcurrentDb fora.** Kernels de grupo pagos; o objecto
  multi-thread (lock-order, flush-vs-rotate, N-way do caller) não.
- **Problem 8 — C1 é eleição, não valor servido.** L28 continua planta.

## Proposed Solution

Um objecto de prova, quatro corolários, métrica que recusa o atalho.

1. **Spec.** Reusar `pedradb-spec` (0166) como spec Lean de estado
   abstracto: prefixo WAL acked, LSM newest-first, leftover TX,
   config (joint = duas maiorias). Dente as-is em cada frase.
2. **Relação de abstracção `Rel`.** Estado dos kernels extraídos
   (WalState, `visible_at`/walk, leftover, membership, OCC last_seq)
   ↔ estado abstracto. Os 326 átomos são passos que preservam `Rel`.
3. **`pedra_refines`.** Simulação: cada plano que o handler de
   produção **matcheia** é um passo da spec. Env (open/read/write/
   `fdatasync`) fica TCB — igual ao assembly do seL4. Não dump de
   `db_kernel.rs` / `concurrent_kernel.rs`.
4. **Quatro closes de produto, sujeito = handler:**
   - **R1** — walk Isolated que o get chama (`get_live` / nome de
     produção): última escrita commitada ou `None` se apagada.
   - **D1** — `put_handler_plan ∘ wal_commit_plan ∘ crash_legal ∘
     recover ∘ get_live`, Env honesto: put `Ok` sobrevive ao reopen.
   - **T1** — recover que matcheia `leftover_fate` + revert: commit
     aplica tudo, abort/crash nada, nenhum intermédio visível ao get.
   - **C1** — valor que uma réplica participante serve ∈ prefixo
     majority-durable da config vigente (joint: ambas).
5. **Get walk Isolated.** Uma fn total no ficheiro que o rustc liga;
   `match` no lookup/get; extract Aeneas desse corpo; sem `dyn
   Iterator` no termo. `visible_at` vira callee, não o sujeito R1.
6. **Gates honestos (uma vez o piso desce, depois só sobe):**
   - compose: crédito só com `unfold` de **duas** `def` Lean
     (caller e callee) + ramo as-is. `String` literal / `m2_fn_*`
     não conta. `native_decide` sem `unfold` não conta.
   - DEFINING A1/A3/A6: o **enunciado** casa o molde 0166 (não o
     identificador). A2a = `kernel_loc / (kernel_loc + handler_loc)`
     do `residuals.json` glue, não o glob de nomes.
   - produto: coluna `subject` = fn que o handler chama para
     put/get/recover/replica-read. Camada `close` recusada se o
     teorema só `unfold` o predicado interno (`visible_at`,
     `leftover_fate`, `wal_commit_plan` sozinho, `joint_election_ok`
     sozinho).

## Delivery slices (mandatory)

### P0 — a métrica deixa de mentir (útil sozinho: o board passa a nomear o próximo teorema de produto)

- [x] **P0.1** Gate `scripts/check_compose_floor.py` (+ `--selftest` +
  job `compose-floor`): m2 conta só teoremas com dual-unfold de duas
  `def`; `ComposeM2.lean` sai do numerador (apagar o ficheiro ou
  deixá-lo sem crédito). Recount honesto no mesmo commit; piso m2
  **desce uma vez** (autorizado) e depois só sobe. Absorve 0220 P0.1
  e 0225 P0.1 (flip `done` no mesmo commit) — status: `done`
- [x] **P0.2** `sel4_gap.py` honesto: A1=1 iff o enunciado de
  `pedra_refines` é simulação Rel (não alias de
  `storage_write_path_recovered_iff`); A3=1 iff o enunciado quantifica
  o walk de get/scan (não só `visible_at × tombstone`); A6=1 iff o
  enunciado cobre lock-order do `ConcurrentDb` (não só OCC membro);
  A2a usa `handler_loc`. Floors DEFINING **descem uma vez** no mesmo
  commit. Print deixa de poder ser 100% com os furos 1–8 abertos —
  status: `done`
- [x] **P0.3** `product_guarantees.tsv`: coluna `subject` (handler fn).
  Checker recusa `close` cujo teorema não `unfold` o `subject`. Freeze
  honesto: R1/T1 continuam `atom` (`visible_at` / `leftover_fate`);
  D1/C1 continuam `close` **de plano** até P1 (não descer camada —
  o `subject` é que fica marcado `plan` até a promoção).
  `leftover_next` passa a nomear **P1.1**. Ledger: uma linha “sujeito
  0227” na tabela de produto — status: `done`

### P1 — as quatro frases sobre o código que corre, e o teorema único

- [x] **P1.1** Walk Isolated de get (produção): fn total no kernel que
  o lookup/get **matcheia** (sem `Box<dyn Iterator>` no termo);
  extract Aeneas; as-is devolve versão hidden/Deletion; planta DST
  nomeada. `visible_at` / `prefer_newer_seq` / `mem_point_decides` /
  `probe_order_covering` são callees — status: `done`
- [x] **P1.2** R1 `atom→close`: teorema ∀ sobre o walk P1.1 — get
  devolve a última escrita commitada ou `None` se apagada; nunca
  Deletion/RangeDeletion/hidden. TSV R1 `subject`=essa fn, camada
  `close`. Corolário cita `visible_at` e newest-first; não reabre R1
  como modelo — status: `done`
- [x] **P1.3** D1 sobre put-crash-reopen: dual-unfold
  `put_handler_plan × wal_commit_plan × crash_legal × recover_collect
  × get_live` (Env honesto; `Lying` suspende, dente as-is). Enunciado
  = frase 0166. TSV D1 `subject`=put/reopen path. Não é segundo close
  de `wal_commit_plan` sozinho — status: `done`
- [x] **P1.4** T1 `atom→close`: recover de produção matcheia
  `leftover_fate` + revert; teorema ∀ — leftover aborta e não
  materializa; TX commitada visível ao get walk; abortada/crashed
  não. TSV T1 `subject`=recover, camada `close`. As-is materializa —
  status: `done`
- [x] **P1.5** C1 sobre valor servido: teorema ∀ — numa réplica
  participante, o get walk devolve um valor do prefixo
  majority-durable da config vigente; durante joint, ambas as
  maiorias (cita `joint_election_ok`, `ae_entry`, `propose_ack_ok` /
  apply). TSV C1 `subject`=replica-read. Primeiro par `l28_*`
  **teorema** só se o enunciado for esta frase (0187 P2.1 levantado
  neste ponto; as outras 32 plantas ficam campaign) — status: `done`
- [x] **P1.6** `pedra_refines` único: `Rel` + os quatro corolários
  P1.2–P1.5; zero `sorry`; A1=1 só com este enunciado. Apagar os
  aliases de `ComposeDefining.lean` (`pedra_refines` /
  `confinement` / `concurrent_db_write_group_forall` como ticks).
  `lake build` 2× verde. Ledger: as quatro linhas de produto
  apontam para estes teoremas no mesmo commit — status: `done`

### P2 — ConcurrentDb, espinhas que faltam, Inv do engine, trampolim vazio

- [x] **P2.1** Data-race row: dual-unfold `occ_snap_lock_order ×
  wal_rotate_decision` com `commit_inflight` (idle rotate recusado).
  Absorve 0220 P2.1 — status: `done`
- [x] **P2.2** Deadlock row: teorema do **ciclo** 2PL sobre
  `wait_for_deadlock` (não um passo de lookup opaco; não LockBud).
  Absorve 0220 P2.2 — status: `done`
- [x] **P2.3** N-way do handler: `occ_batch_plan × group_validate`
  com N>2 e membro lagging conflita; dual-unfold do plano que
  `validate_occ_batch` chama. Absorve 0220 P1.2. A6 pode tickar
  aqui se P2.1 também estiver pago (lock-order + N-way) — senão A6
  espera P2.1 — status: `done`
- [x] **P2.4** Espinhas 0220 P1 restantes, **uma dual-unfold por
  commit**, slice fecha quando as três estiverem em `Compose*.lean`
  com unfold real: `auto_flush_gate × mem_auto_flush_plan ×
  cf_flush_plan`; `parked_pop_plan × group_ack_plan`;
  `changelog_store_plan × pit_resync_rewrite_plan`. Absorve 0220
  P1.1/P1.3/P1.4 e 0225 P0.2/P1.2/P1.3 — status: `done`
- [x] **P2.5** Inv-WAL / Inv-LSM sobre a **imagem de `Rel`** dos
  estados alcançáveis pelo engine (passos = planos que o handler
  matcheia), citando P1.6. Não re-prova os lemas um-passo do 0198/
  0200 — status: `done`
- [x] **P2.6** Trampolim write-path: os 2 `if`s de `lead`
  (`solo_bypass`, `pending.ops.len() == 1`) chamam kernels
  matriculados (RFC-0226) **e** `leftover_next none` no board.
  `unpaid_trampoline_data_fate_ifs=0`. Sem dump — status: `done`
- [x] **P2.7** Checker: wrap-factory e pares `*_modelo` / `put_ok`
  do 0166 **não** podem ser `subject` de produto nem crédito m2.
  `--selftest` sabota uma linha dessas → vermelho — status: `done`
- [x] **P2.8** Iterator residual nomeado: `probe_order` unpacked
  (`filter.collect`) ou vira Isolated como o covering, ou fica
  linha em `EXTRACT.md` “não é o termo do get” (o get já usa P1.1).
  `inv_lsm` / `level_distinct` (nested return/break) idem: extract
  Isolated **ou** residual publicado. Nada silencioso — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | compose-floor honesto (dual-unfold; ComposeM2 sem crédito) | done | — | 2026-09-15 |
| P0.2 | p0 | sel4_gap honesto (enunciado, não nome; A2a + handler_loc) | done | — | 2026-09-15 |
| P0.3 | p0 | product `subject` + leftover_next P1.1 | done | — | 2026-09-15 |
| P1.1 | p1 | walk Isolated de get (produção + extract) | done | — | 2026-09-15 |
| P1.2 | p1 | R1 close sobre o walk | done | — | 2026-09-15 |
| P1.3 | p1 | D1 put-crash-reopen | done | — | 2026-09-15 |
| P1.4 | p1 | T1 close sobre o recover | done | — | 2026-09-15 |
| P1.5 | p1 | C1 valor servido ∈ prefixo majority-durable | done | — | 2026-09-15 |
| P1.6 | p1 | `pedra_refines` único (Rel + 4 corolários) | done | — | 2026-09-15 |
| P2.1 | p2 | lock-order × rotate (`commit_inflight`) | done | — | 2026-09-15 |
| P2.2 | p2 | ciclo 2PL `wait_for_deadlock` | done | — | 2026-09-15 |
| P2.3 | p2 | N-way OCC do handler | done | — | 2026-09-15 |
| P2.4 | p2 | espinhas auto-flush / parked_pop / changelog | done | — | 2026-09-15 |
| P2.5 | p2 | Inv sobre imagem de Rel (engine) | done | — | 2026-09-15 |
| P2.6 | p2 | trampolim `lead` sem `if` data-fate cru | done | — | 2026-09-15 |
| P2.7 | p2 | wrap-factory / modelos 0166 fora do crédito de produto | done | — | 2026-09-15 |
| P2.8 | p2 | Iterator residual nomeado ou Isolated | done | — | 2026-09-15 |

## Acceptance Criteria

- **Tests**
  - P0.1: `python3 scripts/check_compose_floor.py` GREEN; `--selftest`
    recusa (a) crédito por `def m2_fn_X : String`, (b) teorema sem
    `unfold` de duas `def`, (c) `native_decide` sozinho. m2 live
    **desce** vs 324/326 no mesmo commit; 0220/0225 P0.1 `done`.
  - P0.2: `python3 scripts/sel4_gap.py` imprime A1=A3=A6=0 até P1.6/
    P1.2/P2.1+P2.3; A2a < 100% (denominador inclui `handler_loc`).
    `--gate` GREEN **depois** do reset único dos floors. DEFINING
    print ≠ 100% enquanto P1.6 estiver `todo`.
  - P0.3: `python3 scripts/check_product_floor.py --selftest` recusa
    `close` cujo teorema não unfold o `subject`; descer camada →
    vermelho. `candidates.py leftover_next` nomeia P1.1.
  - P1.1: `cargo test` nomeado dirige a fn de produção; extract em
    `lean_extracts.sh --required`; as-is planta vermelha no dente.
  - P1.2–P1.5: `lake build` do módulo; statement com `∀`; zero
    `sorry`; TSV camada `close` e `subject` = handler; planta as-is.
  - P1.6: um teorema `pedra_refines` cujo enunciado cita as quatro
    frases; `ComposeDefining.lean` sem aliases; A1=1; ledger aponta
    para ele.
  - P2.1–P2.4: dual-unfold caller **e** callee; as-is; m2 só sobe se
    P0.1 contar o teorema.
  - P2.6: `unpaid_trampoline_data_fate_ifs=0`; leftover_next none.
  - P2.7: selftest wrap-factory → RED.
- **Telemetry / Analytics** — none — sinal binário (CI
  vermelho/verde); floors TSV.
- **Documentation** — este RFC (status table no mesmo commit de cada
  fatia); `docs/verification-ledger.md` sujeito 0227; uma linha em
  `docs/status.md`; nota datada em `formal/aeneas/EXTRACT.md` por
  land; 0220/0225 P0.1 flipped no commit P0.1.
- **Screenshots** — none (backend/CI-only).

## Out of scope

- Provar o TCB / dependências (lista na secção seguinte).
  `media_durable_admitted`, `forall_schedules_admitted`,
  `lock_interleavings_admitted` ficam `false`.
  Caminho honesto (não flipar): [RFC-0229](0229-host-tcb-media-sched-pct-stdenv-liveness.md).
- Dump de `db_kernel.rs` / `concurrent_kernel.rs` no prover.
- Re-pin Aeneas/Charon sem widen-sem-`sorry`. Dyn-Trait continua
  recusa medida; Isolated é a rota.
- Montanha FDB recipes (`crates/montanha-fdb-recipes/**`) — portão
  do usuário, intocado.
- As 32 plantas L28 TCP que **não** são o enunciado C1 — continuam
  campaign (0187). TCG power-cut / `F_FULLFSYNC` — experimento
  nightly. Exaustivo N=4 e crash-injection T>12/S>4 — fronteira
  **do harness**, não do `pedra_refines` (alargar é movimento de
  ledger 0187, RFC filho se o runner aguentar).
- Perf / Rocks parity / sync-peer / RFC-0226 meter.
- Mintar `fate_iff` novo como crédito de produto; wrap-factory;
  cartoon Verus (`u64` / `Seq<u8>` ≠ rustc).
- “Somos seL4” / “sem bugs” / “garantia total”.

## Dependências (TCB — fora de prova, por decisão registada)

Estes itens **não** entram neste RFC. Fechá-los é outro projecto
(CompCert, seL4, Ironclad, prova de mídia). Apagá-los do
`never_floor` sem RFC dono é vermelho.

| Dependência | Onde está nomeada | O que *fecharia* (e não vamos) |
|---|---|---|
| CPU / modelo de memória / microcode | `never_floor` `R-cpu` (0061) | prova do silício |
| rustc / LLVM / linker (binário = extract) | `R-rustc`; RFC-0172 TV de *idle IR*, não object↔Lean | translation validation de um alvo pinado é o máximo já recusado como never |
| Verus / Z3 / Lean kernel / Aeneas / Charon | `R-verus`; pins em `proof-check.yml` | provar o prover |
| Stdlib Aeneas (4 `sorry`: `StringIter.lean`, `Slice.lean`) | ledger TCB; pin `daa85d7` | zero-sorry absoluto da toolchain |
| Colisão CRC / hash | `R-crc` | prova criptográfica da função |
| Crates / libs (`lz4_flex`, `parking_lot`, `bytes`, …) | `R-deps` | verificar cada dep |
| Contrato do SO: `fdatasync`/`fsync`/`F_FULLFSYNC` persistem antes de retornar | RFC-0078; `media_durable_admitted=false` | prova de mídia / firmware |
| Firmware / controladora de disco não mente ao SO | RFC-0187 §Out of scope | — |
| Scheduler do SO / `∀π` de interleavings reais | RFC-0070; `lock_interleavings_admitted=false`; PCT é campanha | prova do kernel do host |
| Harness PCT controla grants; threads fora do modelo | RFC-0070 R-pct / R-glue | — |
| `StdEnv` = filesystem real nas campanhas não-sim | `env.rs` | — |
| Unsafe POSIX / io_uring / C-API | residuals `R-unsafe-*` (Miri/ASan, não ∀) | — |
| Liveness sem axioma de quórum-vivo | ES-1/2/3; `liveness_admitted` | fairness + rede real |
| Hardware ECC / silent corruption abaixo do CRC | RFC-0060 | — |

**Leitura:** no fim do P1.6 o produto pode dizer as quatro frases
**relativas a esta tabela**. Isso é a mesma *classe de claim* do
seL4 (prova contra TCB escrito). Não é a mesma *classe de garantia*
(eles escreveram o kernel no prover; nós extraímos decisões do Rust
e esvaziamos o trampolim até só restar Env).
