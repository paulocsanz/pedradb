# RFC-0287: Cinco Fronteiras Matemáticas e Estruturais do Motor Puro PedraDB

- **Data:** 2026-09-27
- **Status:** Aprovado e Registrado para Implementação Imediata
- **Escopo:** Exclusivo PedraDB Pure Storage Engine (Sem Montanha, sem cluster, sem componentes de rede)
- **Restrição de Rigor:** `#![forbid(unsafe_code)]`, Zero-Twin Policy (RFC-0270), Rigor Absoluto (RFC-0273)

---

## 1. Contexto e Motivação

O motor de armazenamento do PedraDB atingiu paridade de desempenho contra o RocksDB e possui verificações formais para concorrência (Miri sob Tree-Borrows, Loom), linearizabilidade e recuperação sob queda. Contudo, uma análise fundamentada em verificação formal avançada (estilo TLA+, Coq e semântica de sistemas de arquivos) revela 5 premissas implícitas não axiomatizadas no núcleo de armazenamento puro:

1. **Topologia de Particionamento sob Entropia Adversarial:** A suposição de que chaves são distribuídas de forma suave, ignorando colapsos de altura do LSM sob cargas fractais ou Zipfianas ($\alpha \ge 2.5$).
2. **Liveness sob Reserva de Emergência em Disco Cheio:** A suposição de que a compactação sempre consegue desalocar espaço antes do `ENOSPC`, ignorando deadlocks de alocação de metadados temporários.
3. **Causalidade Monótona sob Regressão de Relógio de Parede:** O risco de ressurreição de dados ou expiração prematura de TTL quando o relógio físico sofre recuos retroativos ($\Delta t < 0$, NTP step-back).
4. **Coerência de Geração na Borda de DMA/Readahead:** A corrida entre I/O assíncrono de readahead e desalocação/reciclagem de blocos físicos por compactações concorrentes.
5. **Cadeia Criptográfica no WAL sob Corrupções Não-Torn:** A vulnerabilidade de parsers de WAL lineares diante de falhas descontínuas causadas por escrita desordenada de controladoras NVMe.

Esta RFC define os contratos formais, invariantes e kernels executáveis em Rust puro e seguro para neutralizar cada uma dessas vulnerabilidades.

---

## 2. Fronteira 1: Álgebra de Particionamento por $\epsilon$-Aproximação e Entropia de SST

### 2.1 O Problema
Sob distribuições patológicas de chaves (ex.: chaves com prefixos longos idênticos e sufixos hiper-concentrados), os cortes de SST baseados apenas em contadores locais produzem arquivos com desbalanceamento severo:
- Milhares de micro-SSTs com 1 registro cada;
- Ou tabelas gigantescas que concentram 90% das leituras em um único nível.
Isso degrada a busca pontual de $O(\log_T N)$ para $O(N)$ e a compactação para $O(N^2)$.

### 2.2 Invariante Matemático
Seja $K$ uma sequência finita e ordenada de pares chave-valor a ser particionada em $m$ SSTs com tamanho-alvo $B_{\text{target}}$.
O particionador calcula a discrepância ponderada com base no peso acumulado de cada chave $w(k)$ e no fator de entropia $\epsilon \in (0, 1)$:
$$\forall i \in \{1, \dots, m-1\}: \quad (1 - \epsilon) \cdot B_{\text{target}} \le \sum_{k \in \text{SST}_i} w(k) \le (1 + \epsilon) \cdot B_{\text{target}}$$
Garantindo teto estrito na profundidade da árvore $D(LSM) \le \lceil \log_T (N / B_{\text{min}}) \rceil + 1$ e ausência de overlaps horizontais dentro do mesmo nível $L_j$.

### 2.3 Kernel de Produção
`crates/pedradb-core/src/sst_topological_entropy_kernel.rs`

---

## 3. Fronteira 2: Aciclicidade de Desalocação e Liveness sob Reserva de Emergência em ENOSPC Dinâmico

### 3.1 O Problema
Em discos com $>99\%$ de ocupação, sistemas de arquivos POSIX (ext4/XFS/APFS) podem recusar alocações mesmo para tamanhos mínimos de metadados (ex.: bloco de manifesto ou header de novo SST intermediário). Se a thread de compactação for bloqueada por `ENOSPC` antes de conseguir excluir (`unlink`) o arquivo antigo substituído, o banco entra em **Deadlock Circular de Espaço**:
- Clientes parados em Write-Stall aguardando compactação liberar espaço;
- Compactação parada aguardando espaço livre para gravar metadados de conclusão.

### 3.2 Invariante Matemático
O motor define uma reserva rígida de emergência $\Omega_{\text{reserve}}$ tal que:
$$\Omega_{\text{reserve}} > \max_{op}(\text{MetadataFootprint}(op)) + \text{SstFooterAlloc}$$
O grafo de dependência de alocação de espaço é modelado com duas classes estritas de prioridade:
1. `ClientWrite`: Admissão pausada quando $\text{DiskAvailable} \le \Omega_{\text{reserve}} + \Theta_{\text{backpressure}}$.
2. `ReclaimDrain`: Permissão exclusiva para consumir a reserva $\Omega_{\text{reserve}}$ para completar a transição de estado que libera os arquivos de entrada.
Provando por indução finita que o estado $S_{\text{full}}$ transiciona deterministicamente para $S_{\text{reclaimed}}$ sem intertravamento.

### 3.3 Kernel de Produção
`crates/pedradb-core/src/enospc_drain_headroom_kernel.rs`

---

## 4. Fronteira 3: Causalidade Monótona Não-Zenoniana no MVCC sob Regressão de Relógio

### 4.1 O Problema
Se políticas de TTL ou expiração de versões dependerem diretamente do relógio de parede (`SystemTime`), saltos retroativos provocados por NTP step-back ou migração de VM violam a monotonicidade:
- Dados deletados por TTL podem "ressuscitar" se o relógio voltar para antes do prazo de expiração;
- Leituras com snapshots podem misturar versões de épocas descontinuadas.

### 4.2 Invariante Matemático
Toda relação de visibilidade MVCC e elegibilidade de GC/TTL é indexada por um relógio híbrido monótono não-zenoniano $\mathcal{L} = (\text{LogicalSeq}, \text{EpochBarrier}, \text{MonotonicTick})$:
$$\text{Tick}_{n+1} = \max(\text{Tick}_n + 1, \text{SystemWallClock})$$
O predicado de visibilidade e purga satisfaz o **Teorema da Não-Ressurreição**:
$$\text{IsPurged}(k, \mathcal{L}_1) \land (\mathcal{L}_1 \prec \mathcal{L}_2) \implies \text{IsPurged}(k, \mathcal{L}_2)$$
Imune a qualquer perturbação $\Delta t_{\text{wall}} \in (-\infty, +\infty)$.

### 4.3 Kernel de Produção
`crates/pedradb-core/src/non_zeno_monotonic_clock_kernel.rs`

---

## 5. Fronteira 4: Coerência de Geração na Borda de DMA/Readahead contra Leituras Fantasmas

### 5.1 O Problema
Em regimes de Direct I/O (`O_DIRECT`) com transferências DMA assíncronas, um leitor pode disparar um readahead para blocos de um arquivo SST enquanto a compactação desvincula o arquivo (`unlink`) e o sistema operacional reutiliza os blocos físicos para um novo arquivo. Se o buffer DMA for entregue após a reciclagem de blocos, o leitor observará dados de outra geração ou arquivo.

### 5.2 Invariante Matemático
Cada bloco físico persistido em SST contém um trailer criptograficamente verificado de 128-bits:
$$\text{Trailer} = \langle \text{FileUUID}_{128}, \text{GenerationId}_{64}, \text{BlockIndex}_{32}, \text{CRC32C}_{32} \rangle$$
O leitor mantém o `ActiveVersionToken` correspondente. Na borda do DMA, o kernel valida:
$$\text{ValidDMA}(B) \iff (B.\text{FileUUID} == T.\text{UUID}) \land (B.\text{Gen} == T.\text{Gen}) \land (B.\text{Idx} == \text{ExpectedIdx}) \land \text{VerifyCRC}(B)$$
Qualquer discrepância aborta imediatamente a leitura e invalida a cache de readahead, forçando resolução segura pela versão ativa.

### 5.3 Kernel de Produção
`crates/pedradb-core/src/dma_generation_fence_kernel.rs`

---

## 6. Fronteira 5: Álgebra de Recuperação com Cadeia Criptográfica no WAL sob Corrupções Não-Torn

### 6.1 O Problema
Falhas de energia em controladoras NVMe com write-cache desordenado podem persistir o Bloco 0 e o Bloco 2 com sucesso, deixando o Bloco 1 com dados corrompidos (lixo magnético/elétrico intercalado). Parsers comuns de WAL que apenas avançam até o primeiro erro descartam silenciosamente o Bloco 2 ou, inversamente, tentam pular o Bloco 1, corrompendo a atomicidade transacional de lotes multi-bloco.

### 6.2 Invariante Matemático
O WAL adota uma cadeia de encadeamento criptográfico estrito estilo Merkle linear:
$$H_0 = \text{Seed}_{64}, \quad H_i = \text{CRC32C}(H_{i-1} \parallel \text{SeqNo}_i \parallel \text{PayloadLength}_i \parallel \text{Payload}_i)$$
O processo de recuperação executa uma verificação indutiva provando o **Teorema do Prefixo Fechado Máximo**:
- O estado recuperado é gerado estritamente pela dobra $\text{Fold}(S_0, [B_0, \dots, B_k])$ onde $\forall j \le k, B_j$ referencia exatamente $H_{j-1}$.
- Se for detectado qualquer salto, gap ou hash quebrado em $B_{k+1}$, o motor aciona **Fail-Stop Determinístico com Diagnóstico**, recusando-se a recuperar blocos desconexos posteriores e impedindo o surgimento de estados inconsistentes.

### 6.3 Kernel de Produção
`crates/pedradb-core/src/wal_crypto_chain_recovery_kernel.rs`

---

## 7. Estratégia de Verificação e Gate Soundness

1. **Compilação Segura:** Código 100% puro sob `#![forbid(unsafe_code)]`.
2. **Miri Concurrency Gate:** Execução de testes de concorrência sob o modelo de aliasing **Tree-Borrows** (`-Zmiri-tree-borrows -Zmiri-ignore-leaks`).
3. **Continuous Verification Chain:** Incorporação no Estágio 22 de `scripts/verify_continuous_chain.sh`.
