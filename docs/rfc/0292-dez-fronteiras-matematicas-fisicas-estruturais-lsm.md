# RFC-0292: Dez Fronteiras de Verificação Matemática, Física e Estrutural do LSM PedraDB

- **Status:** Proposto e Implementado
- **Data:** 2026-09-28
- **Autores:** PedraDB Core & Formal Verification Team
- **Escopo:** `pedradb-core` (Motor de Armazenamento Puro Single-Node)

---

## 1. Contexto e Motivação

As verificações anteriores formalizaram propriedades fundamentais de equivalência sequencial, reticulados de merge, bounds de amplificação RUM, invariantes de partição topológica e controle clássico de Lyapunov. Contudo, uma análise sob o rigor máximo de métodos formais e física de armazenamento identifica 10 fragilidades críticas ainda não contratadas formalmente:

1. **Topologia de Quociente de Chaves:** Discrepância entre hashing de bytes de filtros de Bloom e ordenação semântica abstrata sob relações de equivalência de domínio ($\mathcal{K} / \sim$).
2. **Estabilidade Estocástica de Foster-Lyapunov:** Deriva não-linear sob processos de rajada de cauda pesada de Lévy/Pareto com momentos infinitos.
3. **Bisimulação Causal Trans-Crash ($H_{\text{pre}} \sim_C H_{\text{post}}$):** Prova de preservação de ordem parcial causal entre o estado visível imediatamente antes de uma perda catastrófica de energia e o estado reconstruído pós-reboot.
4. **Semianel com Aniquilador de Range Deletes e Merges:** Confluência e avaliação de acumuladores temporais interceptados por range tombstones em compactações estratificadas.
5. **Invariante de Bounded Extent Dispersal:** Teto rígido de fragmentação física de extents no sistema de arquivos POSIX sob repetidos ciclos de `fallocate(PUNCH_HOLE)`.
6. **Quiescência Atômica de Geração em MemTable:** Proteção de tickets de escrita contra omissão silenciosa de dados em congelamentos assíncronos sob desescalonamento severo de vCPU.
7. **Coerência Release-Acquire Trans-Thread:** Causalidade estrita de memória fraca (ARM64) na publicação lockless de buffers de descompressão zero-copy.
8. **Homomorfismo de Medidas em Metadados de SST:** Conservação monoidal de contadores estatísticos em sub-compactações parciais para evitar compaction starvation.
9. **Aciclicidade e Liveness da Rede de Petri dos Background Workers:** Eliminação formal de deadlocks e livelocks na alocação concorrente de FDs, quota de I/O e versão do MANIFEST.
10. **Protocolo Anti-Amnésia da FTL NVMe:** Prevenção e rejeição imediata de reversão silenciosa de blocos lógicos ressuscitados pelo controlador Flash em falhas de energia.

---

## 2. Especificação Formal dos 10 Pilares

### Pilar 1: Homomorfismo Topológico de Normalização Canônica e Espaço Quociente
Seja $(\mathcal{K}, \prec)$ um conjunto de chaves com uma relação de equivalência de domínio $\sim$.
Definimos a projeção canônica $\pi: \mathcal{K} \to \mathcal{K} / \sim$.
Para qualquer par $k_1, k_2 \in \mathcal{K}$:
$$k_1 \sim k_2 \iff \pi(k_1) = \pi(k_2)$$
**Invariante de Preservação de Quociente:**
1. A função de hash do filtro de Bloom deve operar estritamente sobre a projeção canônica:
   $$\text{BloomHash}(k) = \text{Hash}(\pi(k))$$
2. O particionamento de blocos de índice satisfaz a convexidade quociente:
   $$\pi(k_1) \prec_\pi \pi(k_2) \implies \text{BlockIdx}(\pi(k_1)) \le \text{BlockIdx}(\pi(k_2))$$
Garantindo que uma chave normalizada jamais sofra falso negativo em filtros de Bloom nem seja omitida em buscas por blocos.

### Pilar 2: Estabilidade de Foster-Lyapunov sob Ruído de Lévy com Momentos Infinitos
Sob taxas de chegada estocásticas com saltos fractais de Pareto $W_t \sim \text{Pareto}(\alpha, x_m)$ onde $1 < \alpha \le 2$ (variância infinita), o estado do motor é dado pela dívida acumulada $X_t = \langle M_t, S_t \rangle$ (MemTable dirty bytes e L0 SST count).
Definimos a função de Lyapunov sub-quadrática $V(x) = \|x\|^\gamma$ com $\gamma < \alpha - 1$.
**Teorema de Deriva Estocástica:**
$$\mathbb{E}[V(X_{t+1}) - V(X_t) \mid X_t = x] \le -\epsilon + c \cdot \mathbf{1}_C(x)$$
onde $C$ é um conjunto compacto e $\epsilon > 0$. Isso garante que o processo é Harris-recorrente positivo e o tempo de retorno ao equilíbrio $\tau_C$ é finito quase-certamente:
$$\mathbb{P}(\tau_C < \infty) = 1$$

### Pilar 3: Bisimulação Causal Trans-Crash ($H_{\text{pre}} \sim_C H_{\text{post}}$)
Seja $H_{\text{pre}} = (E_{\text{pre}}, \prec_{\text{pre}})$ o poset de eventos visíveis pré-crash e $H_{\text{post}} = (E_{\text{post}}, \prec_{\text{post}})$ o poset reconstruído após a reprodução do MANIFEST e WAL.
**Teorema de Preservação Causal Trans-Crash:**
Existe uma bijeção restrita sobre o subconjunto de eventos duráveis anunciados $E_{\text{ack}} \subseteq E_{\text{pre}}$ tal que:
$$E_{\text{ack}} \subseteq E_{\text{post}}$$
$$\forall e_1, e_2 \in E_{\text{ack}}: e_1 \prec_{\text{pre}} e_2 \iff e_1 \prec_{\text{post}} e_2$$
Nenhuma transação confirmada desaparece e nenhuma precedência causal é invertida pós-crash.

### Pilar 4: Álgebra de Aniquilação Parcial em Semianéis com Interceptação de Range Deletes e Merges
Seja $\mathcal{M}$ o semianel de mutações de uma chave, contendo $\text{Put}(v)$, $\text{Delete}$, e operandos $\text{Merge}(op)$.
Um range tombstone é representado por $R = [s, e) @ t_R$.
Para qualquer chave $k \in [s, e)$ e mutação $m @ t_m$:
$$\text{Apply}(R, m @ t_m) = \begin{cases} \bot \text{ (Aniquilação)}, & \text{se } t_m \le t_R \\ m @ t_m, & \text{se } t_m > t_R \end{cases}$$
**Teorema de Confluência:**
A avaliação em tempo de leitura $\text{Fold}(R \odot \text{Operands})$ é idêntica à avaliação em tempo de compactação estratificada, mesmo quando os operandos de merge estão espalhados entre múltiplos níveis L1 e L2 e o range tombstone reside em L0.

### Pilar 5: Invariante de Bounded Extent Dispersal ($K$-Contiguidade no VFS)
Para qualquer arquivo vLog ou SST de tamanho $L$ submetido a $N_{\text{punch}}$ descartes de bloco, o sistema de extents do sistema de arquivos é representado por uma lista de extents contíguos $\mathcal{E} = \{ (offset_i, len_i) \}_{i=1}^E$.
**Invariante de $K$-Dispersão:**
$$E \le \left\lfloor \frac{L}{\Omega_{\text{min\_chunk}}} \right\rfloor + 1$$
onde $\Omega_{\text{min\_chunk}} \ge 2\text{ MiB}$. Descartes de tamanho inferior a $\Omega_{\text{min\_chunk}}$ são agrupados ou adiados até que formem um intervalo de descarte contíguo, impedindo a fragmentação microscópica da árvore de extents do inode.

### Pilar 6: Quiescência Atômica de Geração em MemTable
A transição de MemTable ativa para Frozen MemTable é governada por épocas de geração $G \in \mathbb{N}$ e um contador atômico de escritores ativos na geração: $\text{ActiveWriters}(G)$.
**Protocolo de Quiescência:**
1. A thread de rotação publica a nova MemTable ativa com geração $G+1$.
2. A MemTable da geração $G$ entra em estado `Quiescing`.
3. O flusher só assume a custódia da MemTable de geração $G$ quando:
   $$\text{ActiveWriters}(G) == 0$$
4. Todos os escritores com tickets emitidos em $G$ completam a inserção na SkipList antes que o flusher compute o corte de chaves do SST.

### Pilar 7: Coerência Release-Acquire Trans-Thread no Cache de Blocos
A publicação de um bloco descompactado no cache de blocos utiliza um ponteiro atômico $\text{AtomicPtr}$.
**Contrato de Memória Fraca:**
- O descompressor executa stores normais no buffer de dados descompactados.
- A publicação da referência do descritor de bloco no cache é executada com ordenação `Release`:
  $$\text{atomic\_store}(\&entry, ptr, \text{Ordering::Release})$$
- A thread leitora acessa a entrada com ordenação `Acquire`:
  $$ptr = \text{atomic\_load}(\&entry, \text{Ordering::Acquire})$$
Isso estabelece uma relação formal de *synchronizes-with* e *happens-before*, garantindo que todos os bytes do payload estejam visíveis para o leitor em arquiteturas fracas (ARM64/Graviton).

### Pilar 8: Homomorfismo de Medidas em Metadados de SST
Seja $\mathcal{S}$ o monoide comutativo de metadados agregados:
$$\mathcal{S} = \langle \text{Bytes}, \text{Records}, \text{Tombstones}, \text{DeadBytes} \rangle$$
com operação de adição vetorial $(\oplus)$ e elemento neutro $\mathbf{0} = \langle 0, 0, 0, 0 \rangle$.
Para qualquer partição disjunta de chaves de um SST em sub-fatias de sub-compactação $\{ SST_1, SST_2, \dots, SST_k \}$:
$$\mathcal{M}\left(\bigcup_{i=1}^k SST_i\right) \equiv \bigoplus_{i=1}^k \mathcal{M}(SST_i)$$
provando a ausência de distorção ou deriva estatística nas métricas de compactação.

### Pilar 9: Aciclicidade e Liveness da Rede de Petri dos Background Workers
O ecossistema concorrente é modelado como uma Rede de Petri de Lugares/Transições:
- Lugares de Recursos: $P_{\text{fd}}$ (file descriptors livres), $P_{\text{quota}}$ (I/O tokens), $P_{\text{manifest}}$ (lock de catalog).
- Transições de Tarefas: $T_{\text{flush}}$, $T_{\text{compact}}$, $T_{\text{blob\_gc}}$, $T_{\text{scrub}}$.
**Teorema de Ausência de Deadlock:**
Todos os sifões da rede contêm pelo menos uma armadilha marcada no estado inicial $M_0$. Pelo Teorema de Commoner, a rede de Petri é **estruturalmente viva e livre de deadlocks**.

### Pilar 10: Protocolo Anti-Amnésia da FTL NVMe
Cada bloco persistido em mídia carateriza-se por uma tupla de geração física:
$$\mathcal{T}_{\text{block}} = \langle \text{BootUUID}_{128}, \text{Epoch}_{64}, \text{LsmSeq}_{64}, \text{BlockIdx}_{32} \rangle$$
armazenada no trailer de cada bloco físico e assinada com CRC32C.
Durante a leitura ou scrubbing, o motor verifica:
$$\text{Block.LsmSeq} \ge \text{Catalog.MinActiveSeq}(\text{FileId})$$
Se a FTL do NVMe reverter o LBA para uma versão anterior válida antes do erase cycle, o $\text{LsmSeq}$ antigo violará a monotonicidade ativa, resultando em rejeição imediata com erro de `StaleBlockRollbackDetected`.

---

## 3. Plano de Implementação

Todos os 10 pilares serão codificados em `crates/pedradb-core/src/` com `#![forbid(unsafe_code)]`, exportados em `lib_kernel.rs`, testados em `rfc0292_dez_fronteiras_matematicas_lsm.rs`, verificados sob Miri Tree-Borrows e integrados na Cadeia Contínua de Verificação como Estágio 27.
