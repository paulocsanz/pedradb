# RFC-0294: Dez Fronteiras Fundacionais de Verificação Matemática, Física e Estrutural do LSM PedraDB

- **Status:** Proposto e Implementado
- **Data:** 2026-09-28
- **Autores:** PedraDB Core & Formal Verification Team
- **Escopo:** `pedradb-core` (Motor de Armazenamento Puro Single-Node)

---

## 1. Contexto e Motivação

As etapas anteriores de verificação (RFC-0278 a RFC-0292) comprovaram propriedades cruciais de estabilidade de Lyapunov, semianéis de merge, congruência de quocientes e modelos de concorrência fraca. No entanto, sob o escrutínio de teóricos de bancos de dados e pesquisadores de métodos formais de armazenamento, 10 vulnerabilidades fundacionais adicionais foram identificadas na interação entre geometria de dados, concorrência no nível de hardware e dinâmicas de múltiplos níveis:

1. **Monotonicidade de Restart Points sob Compressão Delta:** Garantia de que buscas binárias dentro de blocos de dados de SSTables decodificam chaves sem depender de estado prévio e preservam rigorosamente a ordem lexicográfica.
2. **Paradoxo do Vácuo de Tombstones (Leveled Tombstone Vacuum Paradox):** Prevenção de paralisia não-ergódica do garbage collector quando grandes exclusões derrubam o tamanho em bytes dos níveis e impedem compactações baseadas em limiares de tamanho.
3. **Preservação de Visibilidade Estável de Snapshot sob Compactação:** Blindagem formal de iteradores de snapshot de longa duração contra reciclagem de tombstones e reaparecimento de chaves durante compactações parciais.
4. **Álgebra de Codificação Injetiva de Chaves Compostas:** Framing livre de prefixo (*prefix-free framing*) garantindo que a concatenação de tuplas impede ataques de injeção de delimitador e preserva o isolamento multi-tenant.
5. **Tagging Generacional contra ABA em Memória de Arenas:** Eliminação de corridas em nível de hardware causadas pela reciclagem de buffers de memória em MemTables sem locks.
6. **Desacoplamento Espectral de Compactações Acopladas:** Amortecimento assintótico de ondas de choque e erradicação de ressonância harmônica entre compactações simultâneas de níveis adjacentes (L0 $\to$ L1 $\to$ L2 $\to$ L3).
7. **Fecho Topológico de Inodes em Hot-Backup:** Garantia de que checkpoints físicos assíncronos geram diretórios que são modelos completos e causalmente consistentes da especificação sem vazamentos de referências.
8. **Autômato Histerético de Partição de Cache de Blocos:** Blindagem matemática com curva sigmoide que impede que varreduras analíticas volumosas (scans) expulsem índices e filtros essenciais à latência de consultas pontuais.
9. **Isolamento Epocal de Column Families no WAL Compartilhado:** Eliminação de ressurreição semântica cruzada de dados pertencentes a encarnações anteriores de famílias de colunas recriadas após um drop.
10. **Leases Temporais de I/O com Revogação Pré-Syscall:** Cancelamento atômico de créditos de rate limit quando threads sofrem preempção prolongada pelo kernel ou roubo de vCPU por hypervisors, impedindo rajadas fantasmas no barramento físico.

---

## 2. Especificação Formal dos 10 Pilares

### Pilar 1: Bijeção Estrita de Compressão Delta e Monotonicidade em Restart Points
Seja um bloco de dados SST particionado em $M$ intervalos definidos por restart points $\{ R_0, R_1, \dots, R_{M-1} \}$.
Cada chave $K_{i, j}$ no intervalo do restart point $R_i$ é codificada como:
$$\mathcal{E}(K_{i, j}) = \langle \text{SharedLen}(K_{i, j}, K_{i, j-1}), \text{UnsharedLen}, \text{Suffix} \rangle$$
com $K_{i, 0}$ tendo $\text{SharedLen} = 0$.
**Teorema da Invertibilidade em Restart Points:**
Para qualquer restart point $R_i$, a decodificação $\mathcal{D}(R_i, j)$ depende exclusivamente dos bytes de $R_i$ e do sufixo até o índice $j$:
$$\mathcal{D}(R_i, j) \equiv K_{i, j}$$
e a busca binária indexada exclusivamente pelos restart points satisfaz:
$$R_i \le K < R_{i+1} \implies \text{BinarySearch}(K) = i$$
provando a ausência de chaves quiméricas ou leituras corrompidas independentemente do histórico anterior.

### Pilar 2: Invariante de Drenagem Ativa sob o Paradoxo do Vácuo de Tombstones
Seja $S_L$ o conjunto de registros em um nível $L$. A densidade de tombstones é dada por:
$$\rho_{\text{tomb}}(L) = \frac{\sum_{r \in S_L} \mathbf{1}_{\text{Tombstone}}(r)}{|S_L|}$$
e a idade do tombstone mais antigo é $\Delta t_{\text{max}}(L) = t_{\text{curr}} - \min_{r \in \text{Tombstones}(L)} t_r$.
**Axioma de Ergocidade da Compactação:**
Uma compactação é disparada se e somente se:
$$\text{TriggerCompaction}(L) \iff \text{SizeBytes}(L) > \text{Threshold}(L) \lor (\rho_{\text{tomb}}(L) > \rho_{\text{limit}} \land \Delta t_{\text{max}}(L) > \Delta_{\text{ttl}})$$
Garantindo que $\lim_{t \to \infty} SA(t) = 1.0$ e provando a eliminação de dados mortos mesmo sem novas escritas no nível.

### Pilar 3: Cobertura Temporal Fechada de Iteradores sob Compactação Concorrente
Seja $I_{\text{snap}}$ um iterador de snapshot aberto com sequência $seq_{\text{snap}}$.
Seja $\mathcal{T}_{\text{active}}$ o conjunto de tombstones presentes no banco.
**Invariante de Preservação de Visibilidade:**
O compactador é proibido de purgar qualquer tombstone $T @ seq_T$ que cubra uma chave $k$ se:
$$seq_T \le seq_{\text{snap}} \land \exists SST_i \in \text{VisibleVersionSet}(I_{\text{snap}}): k \in \text{Range}(SST_i)$$
provando formalmente que $I_{\text{snap}}$ mantém uma visão idêntica e confluente de todo o espaço de chaves do início ao fim de sua existência.

### Pilar 4: Álgebra de Codificação Injetiva de Tuplas (Order-Preserving Prefix-Free Framing)
Seja $T = \langle c_1, c_2, \dots, c_k \rangle$ uma tupla composta por $k$ componentes binários arbitrários.
A codificação injetiva $\Phi(T)$ aplica uma transformação livre de prefixo onde cada componente $c_i$ é codificado em blocos de tamanho fixo com bytes de continuidade:
$$\Phi(T) = \phi(c_1) \parallel \phi(c_2) \parallel \dots \parallel \phi(c_k)$$
**Teorema do Isomorfismo Lexicográfico:**
$$\Phi(T_1) = \Phi(T_2) \iff T_1 = T_2$$
$$T_1 \prec_{\text{tuple}} T_2 \iff \Phi(T_1) \prec_{\text{lex}} \Phi(T_2)$$
Garantindo imunidade contra injeção de delimitadores e preservando o isolamento estrito entre tenants em partições compostas.

### Pilar 5: Tagging Generacional de Arenas e Imunidade ao Problema ABA
Cada página em uma arena de memória reciclada é descrita por um ponteiro generacional:
$$P = \langle \text{GenerationId}_{64}, \text{PageIndex}_{32}, \text{Offset}_{32} \rangle$$
O contador global de gerações $\mathcal{G}_{\text{arena}}$ é incrementado atomicamente a cada ciclo de reciclagem pós-flush.
**Invariante de Imunidade ABA:**
Qualquer leitura desreferenciando $P$ valida:
$$\text{Deref}(P) = \begin{cases} \text{Ok}(\&Payload), & \text{se } P.\text{GenerationId} == \mathcal{G}_{\text{current}}(\text{PageIndex}) \\ \text{Err}(\text{StaleGenerationRef}), & \text{caso contrário} \end{cases}$$
provando que leitores retardados detectam imediatamente a obsolescência de páginas recicladas e jamais lêem mutações de novas transações.

### Pilar 6: Desacoplamento Espectral e Amortecimento de Ressonância Harmônica
O sistema de compactações em múltiplos níveis é modelado como um sistema dinâmico linear estocástico em tempo discreto:
$$\mathbf{D}_{t+1} = \mathbf{A} \mathbf{D}_t + \mathbf{W}_t$$
onde $\mathbf{D}_t \in \mathbb{R}^K$ é o vetor de dívidas de compactação dos níveis $L_0, L_1, \dots, L_{K-1}$ e $\mathbf{A} \in \mathbb{R}^{K \times K}$ é a matriz de acoplamento inter-nível.
**Teorema do Amortecimento Espectral:**
O controlador de escalonamento garante que o raio espectral de $\mathbf{A}$ satisfaz:
$$\rho(\mathbf{A}) = \max_i |\lambda_i(\mathbf{A})| \le \gamma < 1.0$$
provando matematicamente que qualquer perturbação de escrita decai exponencialmente para zero e nenhuma onda de ressonância harmônica estacionária pode se formar.

### Pilar 7: Fecho Topológico de Hard-Links em Hot-Backup
Seja $\mathcal{V}_t = \langle \mathcal{M}_t, \mathcal{S}_t \rangle$ a versão ativa no instante de corte $t$, onde $\mathcal{S}_t = \{ s_1, s_2, \dots, s_n \}$ é o conjunto de arquivos SST referenciados pelo catálogo MANIFEST $\mathcal{M}_t$.
**Invariante de Fecho Topológico:**
O diretório de backup $\mathcal{B}$ gerado pelo processo de hot-backup satisfaz:
$$\mathcal{S}_t \subseteq \mathcal{B} \land \mathcal{M}_t \in \mathcal{B}$$
Mesmo sob compactações concorrentes que desvinculem ou criem novos arquivos durante a cópia, a barreira de lease de arquivos impede o unlink de qualquer $s \in \mathcal{S}_t$ até a conclusão do checkpoint, provando a consistência transacional do backup.

### Pilar 8: Autômato Histerético de Partição de Cache de Blocos
A memória total do cache $M_{\text{cache}}$ é particionada dinamicamente entre metadados (índices e filtros) $M_{\text{meta}}$ e dados brutos $M_{\text{data}}$:
$$M_{\text{meta}}(t) + M_{\text{data}}(t) \le M_{\text{cache}}$$
Definimos um piso inviolável para metadados $M_{\text{meta}} \ge \Omega_{\text{meta\_floor}} = 0.3 \cdot M_{\text{cache}}$.
A transição de quota sob scans utiliza uma função histerética com amortecimento sigmoide:
$$\Delta M_{\text{meta}} = -\kappa \cdot \frac{1}{1 + e^{-\alpha (\text{HitRatio} - \theta)}}$$
provando que rajadas de leitura sequencial jamais desalocam metadados abaixo do piso $\Omega_{\text{meta\_floor}}$ e eliminando o cache thrashing.

### Pilar 9: Isolamento Epocal de Column Families no WAL Compartilhado
Cada mutação escrita no WAL carrega a quádrupla:
$$\mathcal{W}_{\text{entry}} = \langle \text{CF\_ID}_{32}, \text{CF\_Incarnation}_{64}, \text{Seq}_{64}, \text{Payload} \rangle$$
O catálogo MANIFEST mantém a tabela de encarnações ativas $\mathcal{I}: \text{CF\_ID} \to \text{ActiveIncarnation}$.
**Teorema de Rejeição de Encarnações Mortas:**
Durante a recuperação pós-crash do WAL:
$$\text{Replay}(e) = \begin{cases} \text{Apply}(e), & \text{se } e.\text{CF\_Incarnation} == \mathcal{I}(e.\text{CF\_ID}) \\ \text{Discard}(e), & \text{se } e.\text{CF\_Incarnation} < \mathcal{I}(e.\text{CF\_ID}) \end{cases}$$
provando que mutações pertencentes a encarnações antigas de uma Column Family que sofreu drop jamais ressuscitam na nova encarnação.

### Pilar 10: Leases Temporais Atômicos de I/O com Revogação Pré-Syscall
Antes de emitir uma operação de I/O, o flusher ou compactor adquire um lease temporal:
$$\mathcal{L} = \langle \text{TokenId}, \text{BytesAllowed}, \text{AcquiredAt}, \text{MaxDurationNs} \rangle$$
Imediatamente antes de invocar a syscall de escrita/leitura física (`pwrite`/`io_uring_enter`), a thread executa a barreira atômica:
$$\text{ElapsedTime} = \text{Clock::now}() - \mathcal{L}.\text{AcquiredAt}$$
$$\text{GuardBarrier}(\mathcal{L}) = \begin{cases} \text{ExecuteIo}(), & \text{se } \text{ElapsedTime} \le \mathcal{L}.\text{MaxDurationNs} \\ \text{AbortAndReacquire}(), & \text{se } \text{ElapsedTime} > \mathcal{L}.\text{MaxDurationNs} \end{cases}$$
provando que threads congeladas por preempção do kernel ou roubo de vCPU abortam leases expirados, eliminando rajadas ilegais fora da cota de largura de banda.

---

## 3. Plano de Implementação

Todos os 10 pilares serão codificados em `crates/pedradb-core/src/` com `#![forbid(unsafe_code)]`, exportados em `lib_kernel.rs`, testados exaustivamente em `rfc0294_dez_fronteiras_fundacionais_lsm.rs`, validados sob o verificador Miri sob *Tree-Borrows* e integrados à Cadeia Contínua de Verificação como Estágio 28.
