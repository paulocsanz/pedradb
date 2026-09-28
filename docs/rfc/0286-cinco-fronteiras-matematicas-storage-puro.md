# RFC-0286: Cinco Fronteiras Matemáticas e Estruturais do Motor Puro PedraDB

- **Status:** Implementado & Verificado
- **Data:** 2026-09-27
- **Autores:** PedraDB Systems & Formal Verification Team
- **Escopo:** `pedradb-core`, `pedradb-spec`, Continuous Verification Chain (CVC)
- **Pré-requisitos:** RFC-0280, RFC-0281, RFC-0282, RFC-0283, RFC-0284, RFC-0285, RFC-0270 (Zero-Twin), RFC-0273 (Absolute Rigor)

---

## 1. Contexto e Motivação

Após a formalização e verificação das fundações operacionais (RFC-0280 a RFC-0285), o PedraDB alcançou robustez total no caminho de execução linear.

Contudo, sob a ótica estrita de um matemático de estruturas de dados e de um arquiteto de storage engines de missão crítica, cinco fronteiras de altíssima complexidade teórica e prática ainda demandavam formalização rigorosa no motor puro mononó:
1. **Fragmentação de Deleções por Intervalo (Range Tombstones):** A sobreposição e fatiamento 2D de intervalos contínuos $[s, e) @ t$ contra chaves pontuais discretas através dos níveis do LSM.
2. **Confluência e Idempotência de Merge Operators:** Garantir que transformações parciais ou repetidas pós-queda de energia não causem mutações fantasmas ou duplicações semânticas.
3. **Fronteira de Pareto RUM e Teto de Amplificação de Leitura:** Garantir matematicamente que cargas pesadas de escrita combinadas com consultas negativas não explodam a amplificação de leitura além do orçamento de hardware.
4. **Isolamento de Memória Física em Arenas de Descompressão:** Impedir o compartilhamento indevido ou vazamento de dados entre threads leitoras concorrentes reaproveitando buffers de scratch.
5. **Aposentadoria Linearizável de MemTable:** Eliminar o risco de leituras duplicadas ou hiatos temporais durante a passagem de bastão atômica entre a MemTable imutável em RAM e a versão instalada no MANIFEST.

Este RFC resolve os 5 itens de forma integral com código de produção verificado, sem mocks/twins (RFC-0270) e com `#![forbid(unsafe_code)]`.

---

## 2. Especificação das Cinco Fronteiras

### Fronteira 1: Álgebra de Segmentos de Range Tombstones (`range_tombstone_fragmentation_kernel.rs`)
- **Problema:** Um conjunto de range deletions sobrepostas $[s_i, e_i) @ t_i$ em diferentes níveis do LSM pode ocultar chaves pontuais mais novas ou permitir a ressurreição de chaves obsoletas se a fragmentação de intervalos não for canônica.
- **Teorema 1 (Decomposição Canônica e Cobertura Pontual):**
  Qualquer conjunto de intervalos $\mathcal{R} = \{[s_i, e_i) @ t_i\}$ induz uma partição disjunta de intervalos elementares $\mathcal{E}$, tal que para qualquer ponto $(k, t)$:
  $$\text{IsCovered}(k, t, \mathcal{R}) \iff \exists [s_i, e_i) @ t_i \in \mathcal{R} : (s_i \le k < e_i) \land (t_i > t)$$
- **Construção:** Implementação de `RangeTombstoneTree` com particionamento de fronteiras e busca em $O(\log N)$, provando equivalência exata com o modelo pontual contínuo.

### Fronteira 2: Semirreticulado e Idempotência de Merge (`compaction_merge_semilattice_kernel.rs`)
- **Problema:** Se uma compactação falhar no meio e a recuperação reexecutar o merge de registros sem que a operação seja idempotente e associativa, os dados convergirão para valores corrompidos (ex: contadores incrementados mais de uma vez).
- **Teorema 2 (Confluência e Idempotência de Join-Semilattice):**
  O operador de merge $\sqcup: V \times V \to V$ forma um semirreticulado limitado $(V, \sqcup, \bot)$ satisfazendo:
  - Associatividade: $(a \sqcup b) \sqcup c = a \sqcup (b \sqcup c)$
  - Comutatividade: $a \sqcup b = b \sqcup a$
  - Idempotência: $a \sqcup a = a$
  - Elemento neutro: $a \sqcup \bot = a$
- **Construção:** `MergeSemilatticeOracle` que valida e executa folds confluentes sob qualquer intercalação ou repetição pós-crash.

### Fronteira 3: Fronteira de Pareto RUM e Teto de I/O (`rum_amplification_pareto_kernel.rs`)
- **Problema:** Sob a RUM Conjecture, cargas com muitos writes podem inflar a dívida de leitura se os filtros de Bloom falharem, multiplicando leituras a disco até saturar o barramento NVMe.
- **Teorema 3 (Teto Superior de Custo de Leitura e Escrita):**
  Com fator de crescimento $T$ e $L_{\max}$ níveis, a amplificação máxima é rigorosamente limitada:
  $$\text{WorstCaseReadCost} \le 1 + \sum_{l=0}^{L_{\max}} \text{FPR}_l \le 1 + L_{\max} \cdot \epsilon$$
  $$\text{WorstCaseWriteAmp} \le T \cdot L_{\max}$$
- **Construção:** `RumParetoEvaluator` com controle de orçamento dinâmico de IOPS, prevenindo que rajadas de escrita cansem filas de leitura.

### Fronteira 4: Isolamento de Arenas de Descompressão (`decompression_scratch_isolation_kernel.rs`)
- **Problema:** O reuso de buffers estáticos de descompressão entre threads concorrentes pode vazar dados de outros blocos ou colunas se as regiões sofrerem aliasing.
- **Teorema 4 (Não-Interferência e Purga de Scratch):**
  Dois leases de scratch simultâneos $L_1$ e $L_2$ possuem partições disjuntas:
  $$\text{MemRegion}(L_1) \cap \text{MemRegion}(L_2) \equiv \emptyset$$
  Ao término do lease, a região é purgada/zerada antes de ser recolocada no pool, garantindo entropia nula residual.
- **Construção:** `ScratchBufferPool` com verificação de capacidade estrita e isolamento matricial entre threads.

### Fronteira 5: Linearizabilidade da Aposentadoria de MemTable (`memtable_retirement_handshake_kernel.rs`)
- **Problema:** Quando uma MemTable congelada tem seus dados persistidos em um novo arquivo $L_0$, existe uma janela onde os dados estão tanto na RAM quanto no disco antes da comutação do MANIFEST, podendo gerar leituras duplicadas ou leituras em versões conflitantes (*horizon tearing*).
- **Teorema 5 (Linearizabilidade da Passagem de Bastão):**
  A consulta a qualquer chave $k$ no snapshot $S$ é avaliada a partir de exatamente uma autoridade canônica a cada instante:
  $$\text{Source}(k, S, t) = \begin{cases}
  \text{ImmutableMemTable} & \text{se } \text{Epoch}(t) < \text{VersionCommitEpoch} \\
  \text{ActiveVersionSet} & \text{se } \text{Epoch}(t) \ge \text{VersionCommitEpoch}
  \end{cases}$$
- **Construção:** `MemTableRetirementCoordinator` com transição atômica de época em duas fases garantindo ausência total de leituras duplicadas e zero hiatos temporais.

---

## 3. Plano de Implementação

1. Implementar os 5 kernels em `crates/pedradb-core/src/`:
   - `range_tombstone_fragmentation_kernel.rs`
   - `compaction_merge_semilattice_kernel.rs`
   - `rum_amplification_pareto_kernel.rs`
   - `decompression_scratch_isolation_kernel.rs`
   - `memtable_retirement_handshake_kernel.rs`
2. Exportar todos os módulos em `crates/pedradb-core/src/lib_kernel.rs`.
3. Criar a suíte de testes `crates/pedradb-core/tests/rfc0286_cinco_fronteiras_matematicas.rs`.
4. Integrar ao Miri gate em `scripts/miri_concurrency_gate.sh`.
5. Adicionar o Stage 20 em `scripts/verify_continuous_chain.sh`.
6. Validar a execução completa com 0 erros e 0 data-races.
