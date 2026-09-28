# RFC-0289: Cinco Fronteiras Matemáticas Avançadas do LSM PedraDB

- **Data:** 2026-09-27
- **Status:** Aprovado e Registrado para Implementação Imediata
- **Escopo:** Exclusivo PedraDB Pure Storage Engine (Sem Montanha, sem cluster, sem componentes de rede)
- **Restrição de Rigor:** `#![forbid(unsafe_code)]`, Zero-Twin Policy (RFC-0270), Rigor Absoluto (RFC-0273)

---

## 1. Contexto e Motivação

O motor de armazenamento do PedraDB fundamenta-se em princípios estritos de invariância física e lógica. No entanto, uma análise formal aprofundada baseada em semântica de sistemas de arquivos, teoria de ordem e sistemas dinâmicos revela 5 fronteiras adicionais que demandam axiomatização matemática:

1. **Preservação de Ordem em Separadores de Índice sob Comparadores Abstratos:** O risco de algoritmos de truncagem de chaves em blocos de índice violarem a transitividade e ordem fraca estrita de comparadores customizados, desvirtuando buscas pontuais.
2. **Alinhamento de Geometria de Bloco em Hole-Punching do VLog:** A necessidade de provar que a desalocação esparsa via `fallocate` opera estritamente em limites alinhados ao sistema de arquivos POSIX ($\mathcal{B}_{\text{fs}}$), impedindo que bytes de valores vivos sejam zerados.
3. **Desacoplamento Determinístico de Pinned Slices contra Leitores Zumbis:** A garantia de que leituras com bloqueio de blocos em memória (`PinSlice`) transicionam deterministicamente para cópia privada (Copy-on-Exceed) ao atingir uma cota de expiração, evitando a retenção infinita de versões e arquivos SST obsoletos.
4. **Convergência de Onda Dinâmica de Compactação por Métrica de Banach:** A prova de que a dinâmica de dívida de compactação entre níveis vizinhos forma uma contração estrita, prevenindo oscilações caóticas e write stalls periódicos.
5. **Unicidade Espaço-Temporal de Nonce em Criptografia de Repouso:** A composição ortogonal de 320 bits garantindo que nenhum bloco criptografado compartilhe nonce sob qualquer sequência finita ou infinita de crashes e reinicializações de nó.

Esta RFC especifica as provas, contratos matemáticos e implementações em Rust seguro para cada uma dessas 5 fronteiras.

---

## 2. Fronteira 1: Oráculo Construtivo de Separadores Monótonos sob Comparadores Abstratos

### 2.1 O Problema
Para comprimir os blocos de índice nos SSTs, motores LSM usam funções de separador mais curto (`FindShortestSeparator(A, B)`). Porém, se o usuário injetar um comparador customizado com semântica especial (ex: prefixos compostos, colations linguísticas ou ordenação numérica), truncar bytes ingenuamente quebra a transitividade da ordem fraca estrita. Isso faz com que uma chave vá parar no bloco SST errado e suma completamente das buscas pontuais.

### 2.2 Invariante Matemático
Seja $\prec$ uma relação de ordem fraca estrita (irreflexiva, assimétrica e transitiva) sobre $\Sigma^*$.
O algoritmo gerador de separador $S = \text{Separator}(A, B)$ satisfaz o teorema de interpolação estrita:
$$A \preceq S \prec B \quad \land \quad \forall x \in \Sigma^*: (A \preceq x \prec B \implies A \preceq S \preceq x \lor x \prec S)$$
Se uma truncagem preliminar violar a ordem do comparador ($S \prec A$ ou $B \preceq S$), o oráculo rejeita a truncagem e retorna $A$ como separador conservador perfeito, garantindo integridade estrita de roteamento.

### 2.3 Kernel de Produção
`crates/pedradb-core/src/separator_order_preservation_kernel.rs`

---

## 3. Fronteira 2: Invariante de Alinhamento de Buracos e Continuidade de Offset no Compacting do VLog

### 3.1 O Problema
No desacoplamento de valores grandes (VLog / BlobDB), a recuperação de espaço em arquivos antigos utiliza `fallocate(FALLOC_FL_PUNCH_HOLE)`. Porém, os sistemas de arquivos POSIX (ext4/XFS) só desalocam blocos físicos inteiros alinhados à sua geometria (geralmente 4 KiB). Se o VLog perfurar buracos em offsets não alinhados, o kernel do SO arredonda os limites de forma não-portável, correndo o risco de zerar silenciosamente bytes de valores vizinhos ainda vivos.

### 3.2 Invariante Matemático
Seja $\mathcal{B}_{\text{fs}}$ o tamanho do bloco físico (4096 bytes).
Para qualquer intervalo de registros mortos $[\text{DeadStart}, \text{DeadEnd}]$, o intervalo seguro de perfuração $[S_{\text{safe}}, E_{\text{safe}}]$ é dado por:
$$S_{\text{safe}} = \left\lceil \frac{\text{DeadStart}}{\mathcal{B}_{\text{fs}}} \right\rceil \cdot \mathcal{B}_{\text{fs}}, \quad E_{\text{safe}} = \left\lfloor \frac{\text{DeadEnd}}{\mathcal{B}_{\text{fs}}} \right\rfloor \cdot \mathcal{B}_{\text{fs}}$$
Se $S_{\text{safe}} \ge E_{\text{safe}}$, nenhum buraco físico é perfurado. Isso garante que nenhum byte pertencente a um valor vivo adjacente jamais se situe dentro do intervalo de descarte.

### 3.3 Kernel de Produção
`crates/pedradb-core/src/vlog_hole_alignment_kernel.rs`

---

## 4. Fronteira 3: Álgebra de Leases Temporais Bounded com Desacoplamento Automático de Pinned Slices

### 4.1 O Problema
Leituras com pinning de bloco (`PinSlice`) eliminam cópias de dados ao manter ponteiros diretos para a cache de blocos. Se uma aplicação cliente abrir um snapshot e esquecê-lo ativo (leitor zumbi), isso impede que os blocos de SST sejam liberados do disco e da memória, causando acúmulo infinito de versões obsoletas e eventual exaustão de descritores e espaço.

### 4.2 Invariante Matemático
Cada lease de leitura de bloco é parametrizada por uma cota máxima de tempo ou transições de época $\Delta_{\text{max\_pin\_ticks}}$.
A máquina de estados de lease opera com transição forçada:
$$\text{State}(L) = \begin{cases} \text{PinnedShared} & \text{se } \text{ElapsedTicks} \le \Delta_{\text{max\_pin\_ticks}} \\ \text{DetachedPrivateCopy} & \text{se } \text{ElapsedTicks} > \Delta_{\text{max\_pin\_ticks}} \end{cases}$$
Ao expirar, o buffer é copiado transparentemente para a alocação privada do iterador e a referência global ao SST é revogada, garantindo liveness estrito da compactação e do GC.

### 4.3 Kernel de Produção
`crates/pedradb-core/src/pinned_slice_lease_kernel.rs`

---

## 5. Fronteira 4: Convergência de Onda Dinâmica de Compactação por Teorema de Contração de Banach

### 5.1 O Problema
Em Leveled Compaction, quando uma rajada contínua de escrita empurra dados de $L_0 \to L_1 \dots \to L_{\text{max}}$, as taxas de compactação entre níveis adjacentes precisam convergir. O que prova que o motor não entra em um regime caótico de "ressonância de compactação", onde a dívida de compactação de $L_i$ oscila com amplitude crescente e causa write stalls catastróficos periódicos?

### 5.2 Invariante Matemático
O vetor de dívidas de compactação $D = \langle d_1, d_2, \dots, d_m \rangle \in \mathbb{R}^m$ evolui segundo o operador de compactação $\mathcal{T}(D)$.
Provamos que sob o fator de amplificação geométrica $\lambda \in (0, 1)$ imposto pelo escalonador:
$$\|\mathcal{T}(D_1) - \mathcal{T}(D_2)\|_{\infty} \le \gamma \|D_1 - D_2\|_{\infty}, \quad \text{com } \gamma = \frac{1}{1 + \text{DampingFactor}} < 1$$
Pelo Teorema do Ponto Fixo de Banach, a sequência de dívidas converge exponencialmente para o ponto fixo estável $D^* = \mathcal{T}(D^*)$, eliminando qualquer risco de amplificação caótica ou live-lock de compactação.

### 5.3 Kernel de Produção
`crates/pedradb-core/src/compaction_banach_contraction_kernel.rs`

---

## 6. Fronteira 5: Prova Construtiva de Não-Reúso de Nonce e Integridade de Criptografia em Repouso

### 6.1 O Problema
Em conformidade com padrões de segurança em repouso (AES-256-GCM / ChaCha20-Poly1305), se blocos SST ou de WAL forem criptografados com contadores de nonce que se reiniciam ou colidem após crashes súbitos e reinicializações de nó, ocorre o catastrófico Nonce Reuse, que permite a atacantes recuperar a chave de autenticação e forjar dados silenciosamente.

### 6.2 Invariante Matemático
Todo nonce criptográfico $\mathcal{N}$ de 320 bits é construído por uma quádrupla ortogonal bijetora:
$$\mathcal{N} = \langle \text{SuperblockUUID}_{128}, \text{EpochBarrier}_{64}, \text{FileNumber}_{64}, \text{BlockOffset}_{64} \rangle$$
Provamos que a função de mapeamento de coordenadas para nonces é injetiva:
$$\forall B_1 \neq B_2: \quad \text{Coords}(B_1) \neq \text{Coords}(B_2) \implies \mathcal{N}(B_1) \neq \mathcal{N}(B_2)$$
Mesmo diante de um ciclo infinito de falhas de energia e reinicializações de nó, a monotonicidade de $\text{EpochBarrier}$ garante probabilidade rigorosamente zero de reuso de nonce.

### 6.3 Kernel de Produção
`crates/pedradb-core/src/crypto_nonce_space_time_kernel.rs`

---

## 7. Estratégia de Verificação e Gate Soundness

1. **Rust Seguro:** Implementado integralmente sob `#![forbid(unsafe_code)]`.
2. **Miri Concurrency Gate:** Verificado sob Tree-Borrows (`scripts/miri_concurrency_gate.sh`).
3. **Continuous Verification Chain:** Integrado como Estágio 24 em `scripts/verify_continuous_chain.sh`.
