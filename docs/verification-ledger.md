# Ledger de verificação — Teorema / Experimento / TCB

**Status:** living (atualizado no mesmo commit que o código que move uma linha de camada)
**ID:** ledger-0187
**Parents:** [0187](rfc/0187-teorema-experimento-tcb.md)
**Gate:** `python3 scripts/check_ledger_consistency.py` (bloqueante; vermelho em inconsistência com `scripts/formal/catalog.json`)

Toda garantia do Pedra mora em uma de três camadas. Este ledger é a lista
autoritativa de qual garantia está em qual camada, com o artefato que a
sustenta. Uma linha só sobe de camada (`experimento → teorema`) com o
gate da camada de destino verde no mesmo commit (three-teeth ou
enumeração completa); nunca por reescrita de ledger.

<!-- ledger-catalog: total=295 proof=262 campaign=33 absent=0 single_artifact=288 aeneas_scripts=228 clones=7 models=34 -->

## Teorema — ∀ sobre código/modelo (machine-checked ou enumeração completa)

| Garantia | Artefato | Piso nomeado (o que NÃO é) |
|---|---|---|
| Kernels de produção verificados (Verus twin / Aeneas Lean) | `scripts/formal/catalog.json` — 261 pares proof; exemplares `catalog:vote`, `catalog:ae_entry`, `catalog:ae_ack`, `catalog:commit_raft`, `catalog:joint_election` | O term de prova é o fonte de produção linkado pelo rustc; twin é gêmeo, não substituto |
| Exaustivo N≤3: todo escalonamento do espaço de grants do harness mantém o invariantes (66/66, 181 nós) | gate P0.1 `crates/pedradb-world/src/bin/gate_exhaustive.rs` | Não é ∀ interleavings do SO (R-pct/R-glue); é ∀ sobre o espaço enumerado do harness |
| Crash-injection em família (RFC-0188 P2.1): todo índice de op falível de cada workload do grid (T,S) recupera fail-closed; T e S medidos no mesmo seam (`FailingEnvArc::tripped`); 4 workloads, T∈{6,9,12} S∈{2,3,4}, 33/33 pontos | gate `gate_crash_injection.rs` (família w0-baseline / w1-double / w2-wide-values / w3-singles) | Fronteira do grid: max T=12, max S=4. Fora: workload com T>12 ou S>4, setor partido/torn write (TCG nightly), ∀π, timing de grupo |
| Piso de barreiras: TODO sítio `sync_data`/`sync_all`/`sync_dir` de produção está pinado (igualdade exata por (arquivo,tipo)) | gate P0.4 `scripts/check_barrier_floor.py` + `scripts/ratchet/barrier_sites.tsv` | A amarração dinâmica prova sync≥1 no stream injetado, não que cada sítio foi exercitado neste run |
| Ratchet de seeds: cada seed pinada reproduz seu desfecho/hash de escalonamento | gate P0.2 `gate_seed_ratchet.rs` + `scripts/ratchet/pct_seeds.txt` | Replay determinístico de seeds pinadas; não é descoberta nem ∀ |
| Escada de profundidade (RFC-0188): extracts 271 + 1 close registrado (`merge_sift_step_repairs_iff`) + 8 atoms (`visible_at_deletion_never_live` ∀ range_hidden — deleção nunca surfaced live; `si_hist_repair_plan_leave_iff_floor_or_match` ∀ tip_gen tip_matches — repair SI-hist deixa o piso gen-0 e o tip já-igual intactos, RFC-0191 P1.5; `apply_put_plan_hist_iff_live_and_gen_positive` ∀ is_reserved si_gen — hist do apply persiste só em chave viva acima do piso gen-0, RFC-0191 P2.3 passo 2; `hist_load_fate_merge_new_iff_decoded_and_not_below` ∀ decoded_ok best_has_user new_last existing — merge da SI-hist na réplica só quando o decode é ok e o novo não fica abaixo do existente (réplica corrompida nunca evita a cópia boa), RFC-0191 P2.3 passo 3; `revert_user_action_restore_value_iff_record_and_present` ∀ had_pre_record pre_was_absent — revert restaura o valor de prepare-time exatamente quando existe registro de pré-imagem marcando a chave presente (registro ausente não é ausência; nunca delete às cegas), RFC-0191 P2.3 passo 4; `txn_commit_action_reverts_iff_abort` ∀ status_is_abort — commit de txn com status abortado sempre reverta e nunca materializa (a cerca de abort sempre ganha, F47), RFC-0191 P2.3 passo 5; `tx_range_action_majority_reverts_iff_failed_and_committed` ∀ range_committed tx_failed — a maioria reverta exatamente quando o TX falhou e o range commitou (TX falho commitado nunca fica vivo na maioria, F47/F34), RFC-0191 P2.3 passo 6; `revert_clears_status_clears_iff_pairs_gone_and_live` ∀ status_is_abort pairs_empty — a chave de status só cai quando todos os pares foram e o status não era abort (a cerca de abort sobrevive a um replay posterior de TxnCommit, F47), RFC-0191 P2.3 passo 7 — meta atom 8/8 fechada) sobre corpos Aeneas extraídos | gate `depth-floor` `scripts/check_depth_floor.py` + registro `scripts/ratchet/close_proofs.tsv` + floors `scripts/ratchet/proof_depth.tsv` | Crédito de degrau só via registro ∀ sem sorry; twins close não registrados contam só no live count do residuals; wrap-factory nunca ganha linha. Extract 276→…→271 é recount das promoções (`visible_at` em 0085e919; `revert_user_action`, `txn`, `tx_glue`, `revert_clears_status` subindo a atom nos passos 4–7 do P2.3 — cada par promovido sai do pool extract), não extração perdida |
| Cobertura de interleaving 15/15 (RFC-0188 P2.2): union das 3 seeds irredundantes (0x1, 0x15, 0xc8) cobre os 15 sítios do inventário, incl. `E.create_open`/`E.remove`/`E.meta`/`W.crash`; remoção de qualquer seed → vermelho | gate `gate_coverage_floor.rs` + `scripts/ratchet/coverage_floor.tsv` | Piso sobre ESTE conjunto pinado com `buggify_widen_sites`; não é ∀ do espaço de seams nem descoberta (soak/hunt noturnos) |

## Garantias de produto (RFC-0191)

Quatro frases sobre o fn que o rustc liga. Camada só sobe
(`model → atom → close`) no **mesmo commit** que o teorema. Gate
`product-floor` (`scripts/check_product_floor.py` +
`scripts/ratchet/product_guarantees.tsv`). Floor: ≥1 atom\|close.

| Frase | Camada | Artefato | Piso nomeado (o que NÃO é) |
|---|---|---|---|
| R1-deleção + value: `Deletion` nunca live; `Value` live iff not hidden; ∀ `range_hidden` | atom | `catalog:visible_at` teorema `r1_get_atom` (`Merge.lean`, unfold `visible_at` nos dois braços); P2.2: lema um passo `inv_lsm_newest_first_never_non_live` (∘ `first_probe_on_equal_lo` 0164) + corolário `r1_get_never_returns_non_live` citando P0.2/P1.1 e o lema | Não é o `get` inteiro; o átomo no path de get. RangeDeletion arm fica no kernel. |
| D1-script: `need_sync ⇒ Sync` antes de Apply/Ok; sync fail ⇒ Fence | close | `catalog:wal_commit_plan` teorema `d1_wal_commit_plan` (`WriteAdmission.lean`, ∀ Bool×Bool, unfold `wal_commit_plan`); P2.1: lema `wal_append_preserves_inv_wal` (um passo Inv-WAL `acked ⊆ synced ⊆ prefixo-recuperável`) + corolário `d1_plan_append_preserves_inv_wal` cita o close e o lema (`WalState.lean`) | Não é prova de `fdatasync`/disco (0078). Close de produto, não segundo close de catálogo (`merge_sift` continua o único registado). |
| T1-leftover: leftover aborta, nunca materializa | atom | `catalog:leftover_txn_is_aborted` teorema `t1_leftover_fate` (`Txn.lean`, ∀ `committed`, unfold `leftover_fate`) | Não é o recover inteiro; o átomo de leftover. Constante `leftover_txn_is_aborted()` agora é `leftover_fate(false)`. |
| C1-joint: eleição joint exige as duas maiorias | close | `catalog:joint_election` teorema `c1_joint_election` (`Membership.lean`, ∀ contagens+`Option` contra a maioria pura `maj`; `majority_of_closed` abre o div/add via specs Aeneas; corolários `c1_old_majority_alone_refuses`/`c1_both_majorities_elect`) | Close de produto; NÃO é segundo close de catálogo (`merge_sift` segue único registado — par já extraído). As-is elege com C-old sozinho (`c1_as_is_elects_on_old_alone` ∀). |

## Experimento — estatística/mecânica (nunca viram um ∀)

| Garantia medida | Artefato | Fronteira honesta |
|---|---|---|
| PCT d=2/d=3/d=4 sobre código real (bug cadeia-3 achado a ~1e-3/seed no d=3; d=2 nunca) | `pct_concurrent.rs` campanhas + ratchet P0.2 | Amostragem; `forall_schedules_admitted` sempre false |
| Lock interleavings / data races | `scripts/race_job.sh` (TSan box no CI) | estatístico, nunca ∀ |
| World swarm 1024/256/256 seeds exit-1 | job `world-parallel` (`synthetic-field.yml`) | campanha; oráculo por run |
| Descoberta de cobertura (sítios de seam) | soak adaptativo + `world-nightly` | o piso P1.1 é tripwire, não descoberta |
| Piso de cobertura pinned-seeds (união 11/15 sítios) | gate P1.1 `gate_coverage_floor.rs` + `scripts/ratchet/coverage_floor.tsv` | 4 sítios (`E.create_open`, `E.remove`, `E.meta`, `W.crash`) ficam com o soak adaptativo |
| Determinismo TCG guest = native | jobs `tcg-*` (`synthetic-field.yml`) | oráculo trace_hash, não wall-clock |
| "Persistiu no disco" (power-cut) | TCG power-cut nightly (P2.2, RFC-0187) + F_FULLFSYNC | SEMPRE experimento; a barreira de SO é TCB |
| Rocks parity (peer default `sync=false`, floor 1.0; G1 fdatasync-antes-do-Ok) | `findings/rocks-parity-floor1x*` | medição; regras de peer do repo |
| Bug-plants three-teeth (dente DST) | `three_teeth_queued.rs` etc. (catálogo, tier campaign — 33 pares `l28_*`) | planta prova que o dente morde, não ∀; exemplares `catalog:l28_durability`, `catalog:l28_tcp_left`, `catalog:l28_tcp_part` |

## TCB — axiomas nomeados (fora de prova, por decisão registrada)

| Axioma | Onde está nomeado |
|---|---|
| Contrato do SO: `fdatasync`/`fsync`/`F_FULLFSYNC` persistem antes de retornar (disk-not-media, never_floor, ∀π fora) | RFC-0187 §TCB; produto G1 assume |
| Firmware/controladora de disco não mente para o SO | RFC-0187 §Out of scope |
| rustc linka o kernel de produção — o term de prova é o binário | RFC-0151 (three-teeth) |
| Pins de toolchain: Verus `0.2026.08.09.92f466f`, Kani sha256, Aeneas `daa85d7`, Charon `0.1.232`/`340b1af`, Lean `4.31.0` | `.github/workflows/proof-check.yml`; re-pin só com widen-sem-sorry medido (P1.4) |
| Harness PCT controla os grants; threads de SO fora do modelo | RFC-0070 (R-pct / R-glue) |
| `StdEnv` = filesystem real do host nas campanhas não-sim | `pedradb-core/src/env.rs` |

## Regras de movimento

1. `experimento → teorema`: só com gate da camada teorema verde no mesmo
   commit (three-teeth completo OU enumeração completa com asserção de
   contagem). A movimentação edita este ledger no mesmo commit.
2. `teorema → experimento` (regressão de camada): gate ficou vermelho e
   o consenso é descer — o commit desce a linha E nomeia o piso perdido.
   Nunca silencioso.
3. TCB novo (axioma novo): precisa de linha nesta tabela com dono e
   motivo; axioma sem nome aqui não existe para o produto.
