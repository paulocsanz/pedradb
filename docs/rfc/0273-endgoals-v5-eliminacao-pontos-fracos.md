# RFC-0273 — Endgoals v5: A Eliminação Integral dos 5 Pontos Fracos da Verificação

**Estado:** ATIVO (Nova régua mandatória para EG2 e EG3)  
**Data:** 24 de Setembro de 2026  
**Parents:** [0272](0272-omnipresent-multi-front-verification.md), [0271](0271-continuous-verification-chain.md), [0270](0270-zero-twin-verification-policy.md), [0260](0260-endgoals-v4-verificacao-pos-adversarial.md)  
**Objeto:** Recalibração dos Endgoals de Verificação Formal (EG2) e Teste Dinâmico (EG3). Proibição expressa de declarar "100%" ou "concluído" enquanto qualquer um dos 5 pontos fracos estruturais permanecer aberto.

---

## 1. Por Que a Declaração de "100%" no EG2 e EG3 Era Ilusória?

A auditoria adversarial de 24 de Setembro de 2026 expôs 5 pontos fracos evidentes que tornavam a alegação de "EG2=100%" e "EG3=100%" cientificamente insustentável:

1. **Mutation Score de apenas 45.00%:** No primeiro teste do motor algorítmico de mutações (`scripts/mutation_fuzzer.py`), 55% dos mutantes gerados sobreviveram. Em `txn_kernel.rs`, 10 de 10 mutantes passaram impunes contra o modelo parcial `txn_model`. Provas formais que toleram mutantes arbitrários são vácuas.
2. **O DST Rodava em RAM (`mem_storage = true`):** O `world_swarm` alcançava 3.800 seeds/s porque operava vetores em memória, contornando chamadas reais a `pwrite`, `fdatasync`, APFS/ext4/XFS e o page cache do kernel. A alegação de robustez física era uma simulação sem contato com o silício.
3. **Composição $m_2$ de Apenas 11.78%:** Das 331 funções atômicas verificadas no Lean 4, apenas 39 eram encadeadas composicionalmente. 88.22% das funções eram "folhas isoladas" sem prova de que a saída de uma alimenta validamente a entrada da próxima.
4. **128.477 Linhas de Código de Cola Fora do Radar (42.09%):** Quase metade do código Rust do repositório (`handler_loc`) não possuía contratos nem verificação, criando um abismo entre os kernels formais e as syscalls POSIX/io_uring.
5. **Intratabilidade Combinatória do Loom:** O teste do Loom com 3 threads levou 25 minutos. Prometer model checking exaustivo no monolito `ConcurrentDb` inteiro via Loom era uma impossibilidade combinatória que forçava a escrita de modelos simplificados.

**Nova Regra de Ouro:** Fica estritamente proibido declarar EG2 ou EG3 como concluídos enquanto essas 5 fraquezas não forem resolvidas de forma integral e mecânica.

---

## 2. A Nova Escada de EG2 v5 (Verificação Formal Contínua & Conectada)

O denominador de EG2 é expandido de 14 para **18 fatias**, incorporando a resolução mandatória dos pontos fracos:

### Bloco A: Soundness e Composição Semântica Real
- **F1 a F3 (Bancadas):** Zero sorries recursivo (auditado em `check_lean_sorries_and_axioms.py`), teto de axiomas congelado, contratos de kernel matriculados.
- **F4 (Composição $m_2 \ge 80\%$ — NOVO):** Elevação da composição semântica em `ComposeM2.lean` de 39/331 (11.78%) para **pelo menos 265/331 ($\ge 80.0\%$)** das funções atômicas encadeadas em teoremas de preservação de estado fim-a-fim ($\text{Admission} \implies \text{Batch} \implies \text{WAL} \implies \text{MemTable} \implies \text{Recovery}$).

### Bloco B: Fechamento da Cola e Redução do TCB
- **F5 (Contratos de Seam na Cola de I/O — NOVO):** Todas as 128.477 LOC de handlers (`handler_loc`) em `pedradb-posix`, `pedradb-io-uring` e `pedradb-core` devem ser guardadas por contratos executáveis de pré/pós-condições (`pedradb-spec`), eliminando transições não auditadas antes de chamadas de sistema.
- **F6 (Eliminação de Código Gêmeo — RFC-0270):** Proibição absoluta de structs mock (como `LoomWriteGroup`). Concorrência verificada diretamente sobre `crate::sync_kernel` na AST de produção.

### Bloco C: Indução Infinita e Ausência de Limites BMC
- **F7 (Verus Ghost State Weaving — NOVO):** Provas de quantificadores universais indutivos ($\forall N \ge 0$) via Verus com `#[cfg(verus_keep_ghost)]` nos kernels centrais (`write_admission_kernel.rs`, `group_commit_kernel.rs`, `wal_buffer_kernel.rs`), eliminando a dependência exclusiva de unwinding finito do Kani ($k \le 8$).
- **F8 a F13 (Bancadas):** Kani bit-precision, Stateright com redução por simetria ($\ge 5$ clientes), catálogo A4 12/12 e invariantes D1/R1/T1/C1.

---

## 3. A Nova Escada de EG3 v5 (Testagem Dinâmica Física em Escala Real)

O denominador de EG3 é expandido de 15 para **18 fatias**:

### Bloco D: Fuzzing de Mutações Algorítmicas (Pilar I)
- **F14 (Mutation Score $\ge 98.0\%$ — NOVO):** O motor [scripts/mutation_fuzzer.py](file:///Users/paulo/software/pedradb/scripts/mutation_fuzzer.py) deve executar contra toda a cadeia contínua ([scripts/verify_continuous_chain.sh](file:///Users/paulo/software/pedradb/scripts/verify_continuous_chain.sh)), atingindo kill-rate comprovado $\ge 98.0\%$. Nenhum mutante em `txn_kernel.rs` ou `write_admission_kernel.rs` pode sobreviver.

### Bloco E: Simulação Física com I/O Real (Pilar II)
- **F15 (DST em Disco Físico Real `PEDRA_SWARM_DISK=1` — NOVO):** O gate oficial de liberação do DST deve rodar campanhas contínuas com `PEDRA_SWARM_DISK=1` sobre sistemas de arquivos reais (ext4 / XFS / APFS), exercitando `pwrite` e `fdatasync` do kernel do SO, sem depender exclusivamente da simulação em memória RAM (`mem_storage=true`).
- **F16 (Crash Replay de Blocos Físicos — CrashMonkey/TCG):** Interceptação de blocos e injeção de torn writes em nível de dispositivo simulado, provando que quedas de energia durante vôo de I/O restauram prefixos duráveis válidos.

### Bloco F: Concorrência Híbrida & Caos Contínuo 24/7 (Pilar III & IV)
- **F17 (Estratégia Híbrida de Concorrência — NOVO):** Divisão formal de escopo:
  - **Loom:** Restrito a primitivas atômicas isoladas ($\le 3$ threads) com permutação C11 completa em `sync_kernel`.
  - **PCT:** Orquestração de threads reais do SO no `ConcurrentDb` com garantia probabilística provada $1/n^d$ (sem explosão de estados).
- **F18 (Daemon 24/7 UCB1 Bandit — [scripts/soak_daemon.sh](file:///Users/paulo/software/pedradb/scripts/soak_daemon.sh)):** Execução autônoma em background acumulando $\ge 10^8$ operações com direcionamento de recompensa UCB1 para sítios de falha raros, com zero silent wrongs registrados.

---

## 4. Recálculo Imediato do Progresso Real

Com a ativação da **Régua v5**:
*   **EG1 (Velocidade):** Permanece **100%** (12/12 fatias done, paridade oficial com RocksDB default preservada).
*   **EG2 (Verificação Formal):** 14 done de 18 fatias $\to$ $\text{piso}(100 \times 14 / 18) = \mathbf{77\%}$. (Pendências: F4 Composição M2 $\ge 80\%$, F5 Contratos de Seam na Cola, F7 Verus expandido).
*   **EG3 (Testagem Dinâmica Concorrente):** 13 done de 18 fatias $\to$ $\text{piso}(100 \times 13 / 18) = \mathbf{72\%}$. (Pendências: F14 Mutation Score $\ge 98\%$, F15 Swarm físico em disco, F17 Estratégia Híbrida formalizada).

**Média Real dos Três Endgoals:**
$$\text{Progresso Real} = \frac{100\% + 77\% + 72\%}{3} = \mathbf{83\%}$$

Nenhum endgoal será considerado concluído até que todas as 18 fatias de EG2 e 18 fatias de EG3 estejam matematicamente comprovadas e auditadas em verde.
