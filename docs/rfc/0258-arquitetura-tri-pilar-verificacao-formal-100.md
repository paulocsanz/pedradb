# RFC-0258 — EG2: Arquitetura Tri-Pilar de Verificação Formal 100% (Aeneas + Kani + Stateright)

**Estado:** ativo (P0/P1 implementados na onda)
**Paga:** EG2 — Verificação Formal de Tudo (66% → 100%)
**Parents:** [0222](0222-escada-ate-o-par-sel4-gap-por-eixo-com-denominador.md) (seL4 gap),
[0227](0227-prova-de-produto-put-get-recover-replica.md) (`pedra_refines` e D1/R1/T1/C1),
[0232](0232-trampolim-so-env-glue-zero.md) (esvaziamento de dados dos trampolins),
[0199](0199-escada-count-bounds.md) (teoremas de custo/trabalho sobre Aeneas),
[0061](0061-residuals-sel4-ironfleet.md) (classe de claim e catálogo de resíduos)

---

## 1. O Problema e a Redefinição do "100%"

Com o fechamento integral da escada de velocidade **EG1 = 100%** (12/12 fatias concluídas ou registradas com teto C formal datado), o gate sequencial comanda o foco na frente ativa de **EG2 — Verificação Formal**.

Na história da verificação de bancos de dados e sistemas de arquivo (IronFleet, seL4, FSCQ, Cogent):
- Abordagens unívocas sofrem de cegueiras estruturais:
  - Provas puramente dedutivas (Aeneas / Lean / Coq) provam teoremas elegantes sobre abstrações extraídas, mas assumem como axiomas o comportamento do hardware, ausência de pânico/overflow em inteiros concretos e ordenamento de memória fraca.
  - Model checkers delimitados (Kani) provam com precisão de bit a ausência de pânico e satisfabilidade de invariantes em código Rust compilado, mas sofrem de teto de profundidade (unwind bound) e não provam indução global.
  - Model checkers de transição de estado (Stateright) exploram exaustivamente interleavings concorrentes, partições de rede e recuperação pós-crash, mas operam em granularidade de ações discretas.

**A Nova Arquitetura Tri-Pilar:**  
O "100% formal" do PedraDB é a **composição ortogonal e sem lacunas** de:
1. **Pilar 1 — Aeneas + Lean 4 (Semântica Funcional e Composição Topo):**
   - Extração do corpo Rust real (`*_kernel.rs`) para termos puros em Lean 4.
   - Teorema topo de refinamento de simulação `pedra_refines` (A1).
   - Teorema de confinamento de leitura `confinement` (A3).
   - Teorema de concorrência global `concurrent_db_write_group_forall` (A6).
   - Espinha de composição de recuperação (`ComposeRecovery.lean`) cobrindo 100% dos átomos de recovery (A4: 12/12).
2. **Pilar 2 — Kani (Precisão de Bit, Ausência de Pânico e Bounded Model Checking na MIR do rustc):**
   - Prova direta sobre código de produção em Rust com harnesses `#[cfg(kani)]`.
   - Bounded verification de ausência de overflow aritmético, integridade de ponteiros e fatias de memória no WAL ticket (`wal_ticket_kernel`), internal keys (`ikey`), e fragmentos de recuperação (`recover_kernel`).
3. **Pilar 3 — Stateright (Concorrência Real, Interleavings e Invariantes sob Crash):**
   - Exploração exaustiva do espaço de estados das máquinas de estado de produção: `WriteGroup` (líder, seguidores, spin vs park, commit em grupo), `recover_kernel` e Raft `joint_leave`.
   - Inclusão mandatória de **dentes anti-vacuidade (mutações AS-IS)**: cada modelo deve refutar a variante defeituosa gerando um contraexemplo concreto.

---

## 2. P0 — Alinhamento dos Pisos Mecânicos e Restauração dos Gates Baratos

Os recentes avanços do caminho de escrita no WAL (RFC-0254/0255) criaram sítios legítimos de barreiras e de transição de kernel nos trampolins.
- `check_barrier_floor.py`: 155 sítios de barreira físicos congelados em `scripts/ratchet/barrier_sites.tsv`. Gate GREEN.
- `check_trampoline_glue.py`: sítios de trampolim atualizados em `scripts/ratchet/trampoline_glue_ifs.tsv`. Gate GREEN.
- Resultado: **9 de 9 cheap gates verdes** (`gates_green = 9/9`).

---

## 3. P1 — Fechamento da Espinha de Recovery A4 (12/12 Átomos)

No eixo 4 do seL4 gap (`scripts/sel4_gap.py`), o denominador de átomos de recovery em `ComposeRecovery.lean` estava em 11/12. O átomo faltante era `reopen_outcome_as_is` (catalog `dictionary_link` / `reopen_outcome_flat_fate_iff`).
A integração formal do átomo na cadeia de composição em `formal/aeneas/lean/ComposeRecovery.lean` eleva a espinha de recovery a:
$$\text{recovery\_atoms\_chained} = 12 / 12 = 100\%$$

---

## 4. P2 — Expansão dos Harnesses Kani no Caminho de Escrita Crítico

Harnesses Kani em `crates/pedradb-core/src/wal_ticket_kernel.rs` e `scripts/kani_wal_ticket.sh` cobrindo:
- `reserve_frame_no_overflow_and_monotonic`
- `pwrite_off_lock_bounds`

---

## 5. P3 — Modelo Stateright da Concorrência do WriteGroup

Implementação de `crates/pedradb-core/tests/write_group_model.rs` modelando:
- Invariante de Linearização de Commit: transações nunca publicam com números de sequência decrescentes ou com buracos não-comprometidos.
- Prova de Anti-Vacuidade: mutação AS-IS que simula liberação de lock fora de ordem é capturada com contraexemplo pelo Stateright.

---

## 6. O que o 100% não é (Recusas e TCB)

- **Não é claim de ausência de bugs no hardware ou OS:** O TCB do hospedeiro (`docs/verification-ledger.md`, RFC-0229) continua formalmente declarado e congelado: `never_floor`, disco $\neq$ mídia (RFC-0078), `fdatasync` executado pelo kernel do SO.
- **Não é prova vazia:** Nenhum teorema sem termo real compilado pelo rustc é aceito como prova de produto.
