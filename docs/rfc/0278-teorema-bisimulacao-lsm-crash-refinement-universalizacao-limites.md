# RFC-0278: Teorema de Bisimulação do LSM, Refinamento de Crash FSCQ-Class, Universalização Lean e Limites de Memória Dinâmica

- **Status:** Proposto e Implementado
- **Data:** 2026-09-24
- **Autores:** PedraDB Formal Verification Core Team
- **Objetivo:** Eliminar as 5 fraquezas teóricas fundamentais identificadas pela auditoria matemática: (1) O "LSM Simulation Gap" via bisimulação indutiva de compactação; (2) Refinamento dedutivo de crash (estilo FSCQ/Perennial); (3) Universalização (∀) e anti-vacuidade das especificações Lean 4; (4) Detecção de UB e data-races via Miri Tree-Borrows; (5) Imunidade a OOM e estouro de pilha nos caminhos críticos de durabilidade.

---

## 1. Motivação e Contexto Teórico

A auditoria matemática adversarial demonstrou que, apesar de 198 arquivos Lean com 0 sorries e 0 axiomas e suites dinâmicas extensivas, cinco pontos cegos teóricos impedem a alegação inequívoca de solidez absoluta:

1. **LSM Simulation Gap:** Provas atômicas em funções isoladas não provam que a árvore multi-camadas $(\text{MemTable}, [\text{Imm}], L_0 \dots L_k)$ refina semanticamente o mapa sequencial $\mathcal{M}: K \to \text{Option}(V)$. Uma compactação com bug de sequence number ou um iterador concorrente poderiam violar linearizabilidade sem violar os teoremas atômicos locais.
2. **Crash Refinement Gap:** Testes de corte elétrico empíricos não substituem o teorema dedutivo de refinamento de crash: qualquer prefixo físico deixado na mídia após interrupção deve recuperar estritamente um prefixo de transações com `fdatasync`, com zero registros fantasmas.
3. **Vacuidade e Instanciação de Constantes em Lean:** Teoremas instanciados em números mágicos literais (ex: `96#u64`) operam como meros testes unitários na lógica formal se não forem quantificados universalmente sobre todo o domínio algébrico ($\forall$).
4. **Semântica Concorrente e Memory Model:** Miri deve auditar ativamente os kernels concorrentes para garantir ausência total de Undefined Behavior e violações de aliasing (Tree Borrows).
5. **Esgotamento de Recursos Finitos:** Garantia de que a engine de durabilidade opera dentro de limites de memória $O(1)$ pós-inicialização e limite estrito de recursão de pilha ($\le \text{MAX\_LEVELS} + 2$) para o `MergeIterator`.

---

## 2. P0: Bisimulação Indutiva do LSM (`lsm_bisimulation_kernel.rs`)

### 2.1 A Função de Abstração $\alpha$
A projeção abstrata mapeia o estado estrutural do LSM para o modelo de dicionário canônico:
$$\alpha(\sigma) = \lambda k. \text{arg max}_{\text{seq}} \{ (v, \text{seq}) \mid (k, v, \text{seq}) \in \text{Mem} \cup \bigcup_{i=0}^k L_i \}$$

### 2.2 Invariante de Sombra (Key Shadowing Monotonicity)
Para qualquer chave $k$ presente em dois níveis $L_a$ e $L_b$ com $a < b$:
$$\text{Seq}(k \in L_a) \ge \text{Seq}(k \in L_b)$$

### 2.3 Teorema de Preservação sob Compactação (Bisimulation Equivalence)
Para qualquer estado $\sigma$, qualquer compactação válida $C: \sigma \to \sigma'$ e qualquer Snapshot $S$:
$$\forall k, \quad \text{Lookup}(\sigma', k, S) \equiv \text{Lookup}(\sigma, k, S)$$
E para qualquer scan intervalar:
$$\text{Scan}(\sigma', k_{start}, k_{end}, S) \equiv \text{Scan}(\sigma, k_{start}, k_{end}, S)$$

---

## 3. P1: Refinamento Mecanizado de Crash (`crash_refinement_kernel.rs`)

Formaliza a relação de transição de crash-recovery:
1. **Monotonicidade Estrita:** Os sequence numbers recuperados formam uma sequência estritamente crescente: $S_{i+1} > S_i$.
2. **Refinamento de Prefixo:** $\sigma_{\text{recovered}} \subseteq \text{AckedTxns}(\sigma_{\text{runtime}})$.
3. **Ausência de Registros Fantasmas:** $\forall r \in \sigma_{\text{recovered}}, r \in \text{PersistedWrites}(\sigma_{\text{runtime}})$.
4. **D1 Durability Guarantee:** Toda transação confirmada via `fdatasync` antes do corte de energia é recuperada intacta.

---

## 4. P1: Universalização e Anti-Vacuidade Lean 4 (`lean_spec_vacuity_gate.py`)

1. **Eliminação de Literais Concretos:** Teoremas que usavam constantes arbitrárias são promovidos a declarações universais $\forall$.
2. **Gate Automatizado de Anti-Vacuidade:** Verifica se as precondições dos teoremas são satisfazíveis e rejeita qualquer teorema cuja hipótese reduza a falso ou cujas conclusões sejam redundantes.

---

## 5. P2: Limites de Memória Dinâmica e Pilha Finita (`bounded_alloc_kernel.rs`)

1. **Orçamento Zero-Allocation no Caminho Crítico:** O writer e os buffers de `fdatasync` operam com alocações pré-reservadas, garantindo zero chamadas a alocador dinâmico de heap durante commits de escritas.
2. **Profundidade Finita de Pilha no MergeIterator:** Prova estrutural de que a profundidade da árvore de mesclagem é limitada superiormente por $\text{MAX\_LEVELS} + 2 \le 16$, impossibilitando estouro de pilha (stack overflow) mesmo em árvores massivas de múltiplos terabytes.

---

## 6. Miri Concurrency Soundness (`scripts/miri_concurrency_gate.sh`)

Executa os kernels atômicos e estruturas de dados concorrentes sob Miri com:
- `-Zmiri-tree-borrows` (verificação do modelo formal de aliasing de ponteiros do Rust)
- Detecção nativa de Data Races entre threads
- Zero tolerância para leituras de memória não inicializada ou referências suspensas.
