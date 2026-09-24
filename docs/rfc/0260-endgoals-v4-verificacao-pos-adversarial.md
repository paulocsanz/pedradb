# RFC-0260 — Endgoals v4: A Nova Régua de Verificação Pós-Auditoria Adversarial

**Estado:** ativo (P0/P1/P2 em execução contínua)  
**Paga:** Refundação Metodológica do EG2 (Verificação Formal) e EG3 (Concorrência Dinâmica)  
**Parents:** [0259](0259-auditoria-adversarial-remediacao-verificacao.md), [0258](0258-arquitetura-tri-pilar-verificacao-formal-100.md), [0222](0222-escada-ate-o-par-sel4-gap-por-eixo-com-denominador.md), [0061](0061-residuals-sel4-ironfleet.md)

---

## 1. Por Que a Régua Anterior Falhou ao Apontar "100%"?

A versão v3 dos Endgoals (`docs/TRAJETORIA.md`) utilizava uma métrica de completude puramente **auto-referencial**:
- 9 de 9 fatias do EG2 foram marcadas como `done` porque todos os denominadores sintéticos da época haviam sido zerados (194/194 arquivos Aeneas matriculados, 12/12 átomos de recovery na espinha A4, 9/9 gates baratos verdes, 309/335 átomos fechados e 26 promovidos por decreto).
- Essa fórmula mecânica cumpriu os gates que haviam sido desenhados, mas gerou a ilusão de "100%" porque os próprios gates possuíam **cegueiras metodológicas**:
  1. O gate de `sorry` olhava apenas `${lib}.lean` e ignorava `${lib}Kernel.lean` (ocultando 3 sorries em `WriteCycleKernel.lean`).
  2. A métrica de teoremas contava definições de reflexividade sintética (`rfl`) em `ComposeM2.lean` como se fossem composição real de sistemas.
  3. O catálogo contava 324 axiomas não provados como se fossem apenas chamadas de biblioteca, sem medir o risco de inconsistência lógica.
  4. O cálculo de LOC formalizada ignorava os 126.548 LOC (42.18%) de código em trampolins e handlers POSIX/rede que de fato processam os dados.
  5. As provas Kani e Stateright rodavam sob limites minúsculos ($k \le 8$, 3 clientes) sem indução geral.

Declarar 100% sob tais premissas é inaceitável para qualquer padrão de rigor científico. O novo EG2 v4 adota a **Régua seL4/IronFleet/VeriBetrKV**.

---

## 2. A Nova Escada EG2 v4 (Pós-Adversarial)

O novo denominador de EG2 é expandido de 9 para **14 fatias rigorosas**, divididas em 4 blocos fundamentais:

### Bloco A: Soundness Lógico & Lean (Fundação Dedutiva)
- **F1 (Zero Sorries Absoluto):** Eliminação incondicional de todo e qualquer `sorry` na árvore inteira (`*.lean` e `*Kernel.lean`), auditado por gate recursivo com código de saída $\ne 0$.
- **F2 (Catálogo e Redução de Axiomas):** Redução mecânica dos 324 axiomas não modelados. Primitivas aritméticas puras (`div_ceil`, `saturating_*`) substituídas por código Lean Std executável e provado. Teto e piso de axiomas restantes congelados em TSV.
- **F3 (Composição Semântica M2):** Elevação de `ComposeM2.lean` de um dicionário de nomes de strings (`rfl`) para uma cadeia real de teoremas relacionando transições de estado: $\text{GroupCommit} \implies \text{WalTicket} \implies \text{DurableRecovery} \implies \text{LinearizableGet}$.

### Bloco B: Fechamento de Superfície & Trampolins
- **F4 (Inventário de Cola & Trampolins):** Manutenção estrita de zero lógica de decisão em trampolins (`check_trampoline_glue.py` GREEN).
- **F5 (Formalização das Chamadas POSIX Críticas):** Formalização algébrica no Lean da semântica de retornos e erros (`EINTR`, `EAGAIN`, `EIO`, `ENOSPC`) para chamadas de `fdatasync`, `pwrite` e `pread`.

### Bloco C: Concorrência Simbólica e Indução Bounded
- **F6 (Expansão do Stateright para Concorrência N-way):** Expansão do modelo `write_group_model.rs` além do limite de 3 clientes através de simetria de redução (verificação de $\ge 5$ clientes e múltiplos ciclos de crash).
- **F7 (Kani com Invariantes Indutivos):** Substituição de bounds puramente finitos por invariantes indutivos em loops de parsing de records do WAL e serialização de chaves internas.
- **F8 (Modelagem Explícita de Relaxed Memory no TCB):** Catalogação formal dos sítios de `Atomic` (`Ordering::Acquire`, `Release`, `Relaxed`) e prova de ausência de corridas de dados no modelo de memória fraca.

### Bloco D: Conservação dos Pilares Anteriores (Bancados)
- **F9 a F14:** Manutenção contínua de D1/R1/T1/C1, escada count RFC-0199, espinha A4 12/12, gates baratos verdes (9/9), e oráculos dinâmicos de concorrência.

---

## 3. Plano de Implementação (P0, P1, P2)

### P0: Soundness Imediato e Gates Recursivos
1. Erradicar os 3 `sorry`s em `WriteCycleKernel.lean` (concluído).
2. Criar `scripts/check_lean_sorries_and_axioms.py` para varredura exaustiva de `formal/aeneas/lean/*.lean`.
3. Integrar a checagem no gate `check_proof_check_toolchains.py` e `sel4_gap.py`.

### P1: Modelagem Executável de Axiomas & Congelamento de Teto
1. Modelar funções aritméticas puras diretamente no Lean (ex: `core.num.U64.div_ceil`), aposentando declarações de `axiom`.
2. Congelar o inventário exato de axiomas restantes em `scripts/ratchet/lean_axioms_catalog.tsv` com gate de proibição de novos axiomas.

### P2: Fortalecimento Semântico no Stateright e Lean M2
1. Ampliar o `write_group_model.rs` no Stateright para permitir escalabilidade concorrente com redução de simetria (até 5 clientes, profundidade maior).
2. Adicionar teoremas de implicação de transição real em `formal/aeneas/lean/ComposeM2.lean`, provando a preservação da barreira de WAL sync através do ciclo de escrita e recuperação.
