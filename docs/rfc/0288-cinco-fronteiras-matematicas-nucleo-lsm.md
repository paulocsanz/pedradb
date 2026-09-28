# RFC-0288: Cinco Fronteiras Matemáticas e Estruturais do Núcleo LSM PedraDB

- **Data:** 2026-09-27
- **Status:** Aprovado e Registrado para Implementação Imediata
- **Escopo:** Exclusivo PedraDB Pure Storage Engine (Sem Montanha, sem federação, sem cluster)
- **Restrição de Rigor:** `#![forbid(unsafe_code)]`, Zero-Twin Policy (RFC-0270), Rigor Absoluto (RFC-0273)

---

## 1. Contexto e Motivação

Com a consolidação das RFCs 0285 a 0287, o PedraDB estabeleceu provas contínuas de confluência, terminação e recuperação sob corrupção de mídia física. No entanto, uma auditoria formal focada em semântica assintótica e integridade algébrica revela 5 pontos críticos no núcleo de execução do LSM:

1. **Amplificação de Descompressão (Zip-Bomb no Kernel):** O risco de payloads com baixíssima entropia expandirem dados em ordens de magnitude superiores à memória disponível, provocando OOM antes do fechamento de leases.
2. **Monotonicidade de Descida em SkipList sob Memória Fraca:** A possibilidade de leitores em arquiteturas ARM64/Graviton observarem ponteiros reordenados entre diferentes níveis da torre de SkipList, saltando sobre chaves existentes.
3. **Homomorfismo de Projeção Vertical em Scans Esparsos:** A necessidade de provar que a leitura seletiva de colunas em SSTs distintos nunca combina valores de Sequence Numbers descompassados (leitura quimera).
4. **Semianel Idempotente de Edições de Versão no MANIFEST:** A garantia algébrica de que replays redundantes ou repetidos de `VersionEdit` convergem para o mesmo estado exato de versão ativa.
5. **Autômato de Inversão de Direção em Iteradores de Fusão:** A preservação estrita de bijeção de cursor e continuidade de chaves durante reversões frequentes entre `Next()` e `Prev()`.

Esta RFC define os modelos formais e kernels de produção para blindar cada uma dessas 5 fronteiras.

---

## 2. Fronteira 1: Invariante de Teto de Descompressão e Streaming Bounded-$M$

### 2.1 O Problema
Mesmo com arenas isoladas de descompressão, se o cabeçalho do bloco indicar um tamanho descomprimido desproporcional ou se o compressor operar de forma não-estratificada, uma entrada maliciosa ou corrompida de 4 KiB pode tentar alocar centenas de megabytes.

### 2.2 Invariante Matemático
A descompressão é governada por um teto de amplificação rígido $R_{\text{max}} = 1024$ e chunks incrementais:
$$\forall B: \quad \text{DeclaredUncompressedSize}(B) \le R_{\text{max}} \cdot \text{CompressedSize}(B) \quad \land \quad \text{ChunkSize} \le 64\text{ KiB}$$
Se o fluxo de saída ultrapassar a razão permitida ou a cota máxima antes de concluir o bloco, a operação aborta com erro imediato `AmplificationLimitExceeded`, garantindo que a memória transitória seja sempre $O(B_{\text{target}})$.

### 2.3 Kernel de Produção
`crates/pedradb-core/src/decompression_expansion_cap_kernel.rs`

---

## 3. Fronteira 2: Bisimulação de Caminho e Barreira de Descida em MemTable SkipList

### 3.1 O Problema
Em SkipLists concorrentes, a torre de ponteiros possui alturas variáveis de $L_0$ até $L_k$. Sob memória relaxada (ARM64), uma leitura que transiciona de $L_{j+1}$ para $L_j$ sem uma barreira de sincronização pode ler um ponteiro mais novo que aterra em uma chave maior que a procurada, saltando sobre a chave correta.

### 3.2 Invariante Matemático
Cada nó da SkipList e cada passo de descida na torre implementa um envelope de barreira monotônica Acquire-Release:
$$\forall (u, v) \in \text{Path}(L_k \to L_0): \quad \text{Key}(u) \le \text{TargetKey} \implies \text{Key}(\text{Descend}(u)) \le \text{TargetKey}$$
Provando indutivamente que a projeção do caminho de busca satisfaz o grafo de acontece-antes (hb), impossibilitando saltos retroativos ou falsos negativos de busca pontual.

### 3.3 Kernel de Produção
`crates/pedradb-core/src/skiplist_weak_memory_barrier_kernel.rs`

---

## 4. Fronteira 3: Homomorfismo Vertical em Scans de Tuplas Esparsas

### 4.1 O Problema
Ao projetar um subconjunto de colunas $C \subset \text{Cols}$ espalhadas em diferentes blocos ou arquivos SST, leituras concorrentes com compactação podem ler uma coluna na versão $\mathcal{V}_1$ e outra coluna na versão $\mathcal{V}_0$, gerando uma tupla quimérica (inconsistente) que nunca existiu no banco.

### 4.2 Invariante Matemático
A máquina de recomposição de tuplas impõe a invariância de snapshot atômico unificado:
$$\pi_C(\sigma_{\mathcal{V}}(T)) \equiv \sigma_{\mathcal{V}}(\pi_C(T))$$
Toda tupla recomposta $T = \langle c_1, c_2, \dots, c_m \rangle$ valida que:
$$\forall i, j: \quad \text{SeqNo}(c_i) \equiv \text{SeqNo}(c_j) \le \mathcal{V}_{\text{snapshot}}$$
Rejeitando e reiniciando a leitura do fragmento se houver divergência temporal entre colunas projetadas.

### 4.3 Kernel de Produção
`crates/pedradb-core/src/sparse_tuple_homomorphism_kernel.rs`

---

## 5. Fronteira 4: Semianel de Transições de Versão e Idempotência de Fechamento no MANIFEST

### 5.1 O Problema
Quedas durante a gravação de checkpoints ou replays de recuperação podem submeter o mesmo `VersionEdit` mais de uma vez. Sem uma álgebra formal idempotente, um arquivo SST pode ser registrado duplicado em dois níveis ou excluído prematuramente.

### 5.2 Invariante Matemático
O conjunto de versões $\mathcal{V}$ munido da operação de transição $\oplus$ e do elemento nulo $\mathbf{0}$ forma um semirreticulado comutativo idempotente:
1. Associatividade: $(V \oplus E_1) \oplus E_2 \equiv V \oplus (E_1 \oplus E_2)$
2. Idempotência: $V \oplus E \oplus E \equiv V \oplus E$
3. Monotonicidade de Epoch: $\text{Epoch}(V \oplus E) = \max(\text{Epoch}(V), \text{Epoch}(E))$
Garantindo que a convergência de estado seja estável sob qualquer permutação ou repetição de deltas válidos.

### 5.3 Kernel de Produção
`crates/pedradb-core/src/manifest_version_edit_semiring_kernel.rs`

---

## 6. Fronteira 5: Autômato de Inversão de Direção em Iteradores de Fusão

### 6.1 O Problema
A alternância de direção em iteradores de merge (MinHeap para `Next()` vs MaxHeap para `Prev()`) sobre múltiplos níveis é fonte clássica de bugs de off-by-one, onde o cursor avança ou recua duas vezes na chave fronteiriça.

### 6.2 Invariante Matemático
O iterador fundido é modelado por um autômato finito com três estados $(Forward, Backward, Neutral)$ e buffer de pivô:
$$\text{Prev}(\text{Next}(it)) \equiv it \quad \land \quad \text{Next}(\text{Prev}(it)) \equiv it$$
A inversão de direção executa uma compensação determinística no cursor subjacente sem consumir I/O adicional, mantendo a bijeção e a ordenação lexicográfica contínua.

### 6.3 Kernel de Produção
`crates/pedradb-core/src/bidi_iterator_reversal_kernel.rs`

---

## 7. Estratégia de Verificação e Gate Soundness

1. **Rust Seguro:** Implementado integralmente sob `#![forbid(unsafe_code)]`.
2. **Miri Concurrency Gate:** Verificado sob Tree-Borrows (`scripts/miri_concurrency_gate.sh`).
3. **Continuous Verification Chain:** Integrado como Estágio 23 em `scripts/verify_continuous_chain.sh`.
