# RFC: prova de fato — D1/R1/T1/C1 como refinamento machine-checked

**Status:** draft
**Updated:** 2026-09-06

## Background

- O catálogo formal atual prova **átomos**: para cada kernel, `exec == spec`
  (+ divergência contra o mutante as-is). Nenhum teorema existente afirma uma
  propriedade **do banco** — "write acked sobrevive a crash" ou "get nunca
  devolve valor apagado" vivem como oráculos de campanha (World/PCT/real TCP),
  não como teoremas. O próprio sistema registra isso: "campaign ≠ ∀π".
- O elo twin↔kernel é heurístico (`check_twins` = token-subset, conjunto sem
  ordem nem multiplicidade; identificadores fora do `TOKEN_RE`). Para ~6
  kernels a extração Aeneas pinando sha256 do arquivo de produção fecha o elo
  de verdade; para os outros ~50 arquivos kernel, "Verus verified" significa
  "verificou uma cópia cujo conjunto de tokens contém o do kernel".
- As provas não rodam no CI: `pedra_formal --ci` executa lint/clones/freeze,
  mas `check_verus` exige `--verus`/`--all` (local). CI verde ≠ prova verde.
- Dois bugs recentes morreram **fora** da camada provada: delete ressuscitado
  (fast path `sorted_by_lo` em `db.rs` — glue; o invariante violado, ordem de
  novidade entre SSTs, existia como teste, não como teorema) e o escape de
  fence ENOSPC mid-flush. Em ambos o kernel foi criado **depois** do bug.
- Números do freeze: 13.014 LOC kernel vs 96.486 LOC handler (~12%/88%);
  2.261 testes; fault grid 42 células decidido (RFC-0165); zero fuzzing
  generativo.

## Problems This Solves

- **Problem:** as propriedades do produto (durabilidade, leitura correta,
  atomicidade de TX, consistência de cluster) não são objetos matemáticos no
  repo — não há o que provar *sobre*, só funções corretas isoladas.
- **Problem:** a prova sobre átomo não impede glue novo incorreto; falta o elo
  átomo → **invariante indutivo** → propriedade (corolário).
- **Problem:** o elo semântico kernel↔twin é heurístico para a maioria dos
  kernels; prova sobre cópia não é prova sobre o binário.
- **Problem:** regressão de prova (Z3 timeout, tweak de spec, binário Verus
  local diferente) não quebra build nenhum.

## Proposed Solution

Quatro camadas, cada uma fechando um elo da cadeia
**propriedade → invariante → átomo → código de produção**:

1. **Specs de máquina (D1/R1/T1/C1)** num crate novo `pedradb-spec`, como
   spec fns Verus sobre estado abstrato, com dentes (a spec deve rejeitar a
   versão fraca as-is de cada propriedade — anti-vacuidade no nível de
   propriedade, não só de função).
2. **Prova sobre produção sem twin:** Kani (BMC sobre o Rust real) para os
   kernels puros de decode/recovery — contrato + ausência de panic provados
   direto no arquivo de produção, sob unwind bounds registrados. Aeneas/Lean
   continua onde já traduz.
3. **Invariantes indutivos:** Inv-WAL (`acked ⊆ synced ⊆
   prefixo-recuperável`) sustenta D1; Inv-LSM (níveis ordenados por novidade,
   runs disjuntos, tombstone domina, MANIFEST == disco) sustenta R1. As seções
   críticas do perfil verificado migram de glue para **exec Verus** com `Env`
   como trait com specs de crash (torn = prefixo; sync = barreira;
   `SyncPolicy::Lying` axiomatizado) — a prova passa a quantificar sobre
   **todo** comportamento do Env.
4. **Refinamento de handler para o consenso:** estender o padrão RFC-0053
   (AE/commit caller refinements) à superfície vote/ae/commit/membership —
   cada handler provado como simulação da transição abstrata do nó; a
   relação L28 deixa de ser wrapper de booleanos e vira invariante de estado
   relacional (C1).

CI passa a implicar prova: jobs Verus+Kani pinados por hash; contabilidade do
catálogo separa **proof objects** de **campaign gates** (família `l28_*`).

### As quatro propriedades (enunciado-alvo)

- **D1 (durabilidade, single-node, perfil verificado):** para toda execução,
  todo `put` que retornou `Ok`, todo crash-point posterior e reopen
  `FailClosed`: a chave está presente pós-reopen com esse valor ou valor de
  escrita posterior também acked. Condicional à honestidade da barreira de
  sync no modelo (`Lying` ⇒ conclusão explicitamente suspensa — herdeiro do
  `fsync_promotes_pending`).
- **R1 (leitura correta):** `get` devolve o valor da última escrita commitada
  (ordem total do log) ou `None` se apagada; nunca valor de escrita
  sobrepujada. Corolário de Inv-LSM.
- **T1 (TX all-or-nothing):** TX commitada aplica todos os efeitos, TX
  abortada/crashed nenhum; nenhum estado intermediário observável pós-reopen.
- **C1 (cluster):** valor servido por qualquer réplica participante pertence a
  um prefixo commitado por majority-durable da config vigente (joint: ambas as
  majorias) — a relação L28 como teorema de refinamento, não gate de campanha.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** Crate `pedradb-spec` com D1+R1 como spec fns Verus sobre
      estado abstruto + dentes (lemma: a versão fraca as-is — "get pré-crash
      basta" / "ordem qualquer serve" — não implica a spec) — status: `done`
- [x] **P0.2** T1+C1 no mesmo crate, mesmo padrão de dente (as-is de T1 =
      "efeitos parciais visíveis"; as-is de C1 = "majoria velha basta") —
      status: `done`
- [ ] **P0.3** Primeira prova sobre produção sem twin: harness Kani no
      `wal/recover_kernel.rs` (`from_record_type`, `fragment_act`,
      `is_length_resyncable`: contrato + não-panic, unwind bound registrado
      no harness) — status: `todo`
- [ ] **P0.4** Job CI noturno `proof-check`: todos `scripts/verus_*.sh` com
      `VERUS` pinado por hash + `cargo kani` nos harnesses existentes;
      regressão de prova quebra build — status: `todo`

### P1 — next wave (a prova de durabilidade D1 no nível modelo)
- [x] **P1.1** `Env` como trait Verus com specs de crash: write parcial
      permitido (torn = prefixo), sync = barreira, `SyncPolicy::Lying`
      axiomatizado como "sync não implica durable" — status: `done`
- [x] **P1.2** Inv-WAL invariante indutivo (`acked ⊆ synced ⊆
      prefixo-recuperável`) provado preservado por append/rotate no nível
      kernel+modelo — status: `done`
- [x] **P1.3** Corolário D1-modelo nomeado (twin Verus; Lean via Aeneas se o
      toolchain traduzir): put Ok ⇒ sobrevive a todo prefixo de torn —
      status: `done`
- [x] **P1.4** Migração das seções críticas write→ack do perfil verificado
      para exec Verus (Env injetado; `profile_report` inalterado; sem
      regressão nas células G1 publicadas) — D1 no nível implementação —
      status: `done` (2026-09-06: `write_ack_kernel.rs` prova append→barrier→ack
      sobre `wal_state_kernel`; twin `verus/write_ack.rs` 12/12 0 err (3×);
      wiring provado em AMBAS as seções críticas do pin — `finish_group_off_lock`
      e `lone_commit`; planta viva `verified_write_ack_on_live_profile_is_not_ok`
      (ledger avança, Inv-WAL vivo, put acked sobrevive a crash+reopen; seam
      lying suspende a premissa); `verified_report` 247/247; custo ledger
      25 ns/grupo durável vs bound 10.000 ns)

### P2 — later / polish (R1, T1, C1 e contabilidade)
- [x] **P2.1** Inv-LSM provado preservado por flush/compact/get/reopen →
      corolário R1 nomeado (a classe do delete ressuscitado vira impossível
      por construção, não apenas detectada) — status: `done`
      (kernel `crates/pedradb-core/src/lsm_r1_kernel.rs` 6/6; twin
      `verus/lsm_r1.rs` 64/64 0 err (2×): Inv-LSM preservado por
      write/flush/compact/reopen via fold lemmas + provIn provenance,
      `r1_theorem` probe == newest sob Inv-LSM, dentes exec probe/version,
      testemunhas nos 3 mutantes AS-IS (probe deepest-first, compact que
      derruba tombstone, reopen reverso) + `r1_modelo_asis` falso no shape;
      planta viva `r1_modelo_on_live_delete_shape_is_not_ok` (put→flush→
      delete→flush→compact→reopen responde None no motor real); 4 pares no
      catálogo: lsm_probe/lsm_compact/lsm_reopen/r1_modelo; real bug
      achado e corrigido no kernel durante a fatia: `lsm_compact` dobrava
      as fontes do mais raso para o mais fundo deixando versões VELHAS
      ganharem — agora mais fundo primeiro, raso sobrescreve)
- [x] **P2.2** T1 como refinamento (fence/abort/recover kernels já existem:
      amarrar em teorema único) — status: `done`
      (kernel `crates/pedradb-store/src/t1_modelo_kernel.rs` 5/5; twin
      `verus/t1_modelo.rs` 8/8 0 err (2×): Inv-TX restaurado por recover
      inclusive de mid-apply, `t1_modelo_theorem` pós-recover T1 vale,
      testemunha mid-apply (staged=2,visible=1) honest T1 / AS-IS leftover
      deixa parcial; átomos amarram `txn_commit_action`+`revert_clears_status`
      (abort keep fence) e `leftover_txn_is_aborted` (recover); planta viva
      `t1_modelo_on_live_abort_reopen_is_not_ok` (2 keys prepared, reopen
      ambas ausentes + status=abort); 3 pares no catálogo:
      tx_abort/tx_recover/t1_modelo)
- [x] **P2.3** C1: refinamento handler↔modelo abstrato para a superfície
      vote/ae/commit/membership (estende RFC-0053) — status: `done`
      (kernel `crates/pedradb-raft/src/c1_modelo_kernel.rs` 4/4; twin
      `verus/c1_modelo.rs` 4/4 0 err (2×): `c1_advance_commit` amarra
      `joint_election_ok`+`may_commit_at`; `c1_modelo` = served ⇒
      `propose_ack_ok` após o passo honesto; testemunha joint-add
      (C-old 2/3, C-new 1/4) honest recusa / AS-IS C-old elege;
      planta viva `c1_modelo_on_live_joint_is_not_ok` (3-node Raft
      propose só acka índice committed); 2 pares no catálogo:
      c1_advance_commit/c1_modelo)
- [x] **P2.4** Contabilidade: catálogo separa proof objects de campaign
      gates; `profile_report` ganha D1/R1/T1/C1 como linhas; residuais
      re-rotulados (R-fsync-lie estreita para "drive físico"; R-glue encolhe
      conforme exec-Verus avança; never_floor inalterado) — status: `done`
      (`object_kinds` + `campaign_prefixes: ["l28_"]`; lint
      `check_proof_vs_campaign`; D1/R1/T1/C1 spec+modelo ON no
      `profile_report`; R-fsync-lie = mídia física pós P1.1–P1.3;
      R-glue cita write_ack/lsm_r1/t1_modelo/c1_modelo; never_floor
      inalterado; `test_proof_vs_campaign.py`)
- [ ] **P2.5** Meta-mutation tests do `pedra_formal.py`: injetar drift
      sintático/semântico num twin clonado e exigir FAIL nomeado (hoje feito
      uma vez à mão; virar teste) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | specs D1+R1 com dentes | done | crate `pedradb-spec` + twin Verus 25/25 0 err (2×) | 2026-09-06 |
| P0.2 | p0 | specs T1+C1 com dentes | done | junto de P0.1 (d1/r1/t1/c1 no catálogo) | 2026-09-06 |
| P0.3 | p0 | Kani sobre recover_kernel de produção | doing | harnesses prontos; gate = run do job proof-check | 2026-09-06 |
| P0.4 | p0 | job CI proof-check (Verus+Kani pinados) | doing | workflow criado; aguardando 1º run verde | 2026-09-06 |
| P1.1 | p1 | Env trait Verus com crash semantics | done | `env_crash_kernel.rs` + twin 15/15 0 err (2×) + planta sim; 6 pares no catálogo | 2026-09-06 |
| P1.2 | p1 | Inv-WAL preservado por append/rotate | done | `wal/wal_state_kernel.rs` + twin 18/18 0 err (2×) + planta sim; 6 pares no catálogo | 2026-09-06 |
| P1.3 | p1 | corolário D1-modelo | done | `d1_modelo_kernel.rs` + twin 14/14 0 err (2×) + planta sim; 2 pares no catálogo | 2026-09-06 |
| P1.4 | p1 | write→ack em exec Verus (D1 implementação) | done | `write_ack_kernel.rs` + twin 12/12 0 err (3×) + planta sim; 3 pares no catálogo; ledger 25 ns/grupo | 2026-09-06 |
| P2.1 | p2 | Inv-LSM → R1 | done | `lsm_r1_kernel.rs` 6/6 + twin `verus/lsm_r1.rs` 64/64 0 err (2×) + planta sim; 4 pares no catálogo; exec teeth `lsm_probe`/`lsm_compact`/`lsm_reopen`/`r1_modelo`; bug real de fold-order corrigido no compact | 2026-09-06 |
| P2.2 | p2 | T1 refinamento | done | `t1_modelo_kernel.rs` 5/5 + twin `verus/t1_modelo.rs` 8/8 0 err (2×) + planta `t1_modelo_on_live_abort_reopen_is_not_ok`; 3 pares tx_abort/tx_recover/t1_modelo | 2026-09-06 |
| P2.3 | p2 | C1 refinamento de handlers | done | `c1_modelo_kernel.rs` 4/4 + twin `verus/c1_modelo.rs` 4/4 0 err (2×) + planta `c1_modelo_on_live_joint_is_not_ok`; 2 pares c1_advance_commit/c1_modelo | 2026-09-06 |
| P2.4 | p2 | contabilidade proof vs campaign | done | catalog `object_kinds`/`campaign_prefixes`; lint proof-vs-campaign; report D1/R1/T1/C1; residuais re-rotulados; `test_proof_vs_campaign.py` | 2026-09-06 |
| P2.5 | p2 | meta-mutation tests do lint | todo | — | 2026-09-06 |

## Acceptance Criteria

- **Tests:** cada slice traz teste/planta nomeada; P0.3–P1.3 trazem teoremas
  nomeados no twin + harness Kani verde no CI; P1.4 exige `verified_report_
  matches_catalog` verde e células G1 publicadas sem regressão
  (`findings/rocks-parity-floor1x-g1/`); P2.1 exige planta viva que o
  corolário R1 torna impossível (o cenário do delete ressuscitado).
- **Telemetry / Analytics:** none — por quê: provas e jobs de CI; o sinal é
  exit code do job `proof-check`, registrado no findings de cada slice.
- **Documentation:** este RFC atualizado no mesmo commit do código; linha no
  `docs/status.md`; `formal/aeneas/EXTRACT.md` quando P1.3/P2.x atingirem
  Lean; residuais (`scripts/formal/residuals.json`) re-rotulados em P2.4.
- **Screenshots:** backend-only.

## Residual impact (mapa)

- **Estreitam:** R-fsync-lie (parte "modelo" vira teorema em P1.1–P1.3; só a
  mídia física resta), R-group-glue (Inv-WAL cobre publish-after-WAL no
  modelo; interleavings de lock/OS continuam), R-glue (encolhe com P1.4/P2.x;
  `db_rs_extracted` segue false — Verus compila Rust, não extrai).
- **Inalterados (never_floor):** R-cpu, R-rustc, R-verus, R-crc, R-deps,
  R-extract. A prova continua **relativa ao TCB** — "prova de fato" =
  teoremas de refinamento com TCB publicado, nunca "correto em absoluto".

## Out of scope

- Extração total do motor para o provador (REFUSE — LEDGER L46, R-extract).
- VerusSync / prova ∀-escalonamentos de `ConcurrentDb` (PCT/TSan seguem).
- Liveness incondicional (axiomas ES-1/2/3 seguem em R-es).
- io_uring ring dentro do modo verificado (R-uring segue).
- Fuzzing generativo e eixos P1/P2 do fault grid (tratados no RFC-0165 e na
  lista de recomendações da auditoria — rastreados à parte, não aqui).
