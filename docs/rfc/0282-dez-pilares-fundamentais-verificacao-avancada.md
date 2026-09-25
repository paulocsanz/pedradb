# RFC-0282: Os Dez Pilares Fundamentais de Verificação Avançada de Sistemas de Armazenamento

- **Status:** Proposto e Implementado
- **Data:** 2026-09-25
- **Autores:** PedraDB Formal Verification Core Team
- **Objetivo:** Resolver de forma integral, matemática e irrefutável os 10 pontos críticos e vulnerabilidades teóricas profundas identificadas na auditoria avançada de sistemas:
  1. **Semântica Formal de Memória Fraca (RC11 / ARM64):** Prova de causalidade e linearizabilidade de sincronização lock-free sob ordens de memória fracas sem depender de agendadores de teste;
  2. **Cura de Escrita Fracionária de Setor Físico:** Envelope de geração em dupla borda para detecção e contenção de torn writes em setores de 512 bytes em discos NVMe reais;
  3. **Partições de Rede Assimétricas e Validade de Leases:** Verificação de matriz dirigida de conectividade e imunidade a leituras sujas sob desvio relativo de relógio físico ($\Delta$-drift);
  4. **Linearizabilidade Estrita de Iteradores de Range:** Prova de snapshot atômico unificado em scans longos sobre trocas concorrentes de SSTables e compactações contínuas;
  5. **Barreira de Não-Divergência de Garbage Collection no VLog:** Oráculo de marcação tricolor provando a impossibilidade de coleta de blobs com referências em vôo ou em staging;
  6. **Terminação e Quiescência de Compactação por Métrica de Lyapunov:** Função de energia estritamente decrescente $E(LSM) \in \mathbb{N}$ demonstrando a ausência de ciclos infinitos de compactação (thrashing);
  7. **Não-Interferência e Zeroização Criptográfica de Memória Residual:** Contrato formal de destruição de entropia em buffers descartados e prevenção de exfiltração em DRAM;
  8. **Agendador de Tempo Denso Contínuo no DST:** Motor de perturbação temporal $\epsilon$-densa para detecção exaustiva de corridas entre temporizadores e eventos de I/O em escala sub-microssegundo;
  9. **Confluência de 2PC sob Queda Concorrente de Coordenador e Participante:** Teorema de convergência atômica provando a ausência de decisões divergentes pós-recuperação;
  10. **Homomorfismo de Esquema e Preservação Semântica Cross-Version:** Álgebra categórica de evolução de esquemas garantindo equivalência estrita de decodificação entre formatos distintos.

---

## 1. Motivação e Rigor Teórico

Um sistema de banco de dados que busca garantias da classe de **seL4, FSCQ e VeriBetrKV** não pode assumir que:
- O hardware executa no modelo sequencialmente consistente (SC);
- Discos físicos gravam blocos de 4096 bytes atomicamente sem falhar no meio dos setores de 512 bytes;
- Falhas de rede são sempre bidirecionais e simétricas;
- Scans de range nunca observam visões temporais deformadas durante migrações de SSTables;
- O Garbage Collector nunca enxerga um blob como morto antes da publicação de seu ponteiro;
- Compactações sempre terminam sem entrar em liveloops;
- Memória desalocada pelo sistema operacional está limpa de dados sensíveis;
- Simuladores discretos cobrem todas as corridas infinitesimais de temporizadores;
- Transações 2PC recuperam de forma consistente sob falhas simultâneas;
- Evoluções de formato de dados preservam a identidade semântica das chaves e valores.

Este RFC estabelece os 10 núcleos de verificação (kernels matemáticos executáveis) para selar definitivamente essas 10 fronteiras.

---

## 2. Especificação Formal dos 10 Pilares

### Pilar 1: Semântica Formal de Memória Fraca (`rc11_relaxed_memory_kernel.rs`)
- **Modelo:** Formalização do grafo de execução $(E, \text{sb}, \text{rf}, \text{mo}, \text{rb}, \text{hb})$.
- **Teorema:** Toda operação de leitura que observa uma escrita através de um par `Release-Acquire` tem sua relação happens-before ($\text{hb}$) estritamente linearizada, garantindo que leituras relaxadas dependentes nunca observem dados não-inicializados em arquiteturas ARM64 fracamente ordenadas.

### Pilar 2: Cura de Escrita Fracionária de Setor Físico (`torn_sector_heal_kernel.rs`)
- **Modelo:** Decomposição de uma página lógica de 4096 bytes em 8 setores físicos de 512 bytes.
- **Teorema:** Um envelope de geração com número de sequência de geração idêntico no primeiro e no oitavo setor, acompanhado de checksums cruzados, detecta com probabilidade $1 - 2^{-64}$ qualquer escrita parcial (1 a 7 setores gravados), rejeitando o bloco como corrompido antes da ingestão.

### Pilar 3: Partições de Rede Assimétricas e Validade de Leases (`asymmetric_lease_kernel.rs`)
- **Modelo:** Matriz dirigida de adjacência de rede $A \in \{0, 1\}^{N \times N}$ onde $A_{ij} \ne A_{ji}$, e relógios locais com desvio máximo $\Delta$.
- **Teorema:** Um nó líder só responde a leituras locais baseadas em lease se o tempo decorrido desde o último quórum satisfaz $t_{\text{elapsed}} + 2\Delta < \text{LeaseDuration}$. Sob qualquer partição unilateral ou tempestade de mensagens, nenhuma leitura stale ocorre.

### Pilar 4: Linearizabilidade Estrita de Iteradores de Range (`range_scan_linear_kernel.rs`)
- **Modelo:** Visão de snapshots multi-níveis sob conjunto de transições de versão $\mathcal{V} \to \mathcal{V}'$.
- **Teorema:** Durante a travessia de um iterador pelo espaço de chaves $[k_{\text{start}}, k_{\text{end}}]$, qualquer substituição concorrente de SSTables preserva a projeção imutável do snapshot $S_{\text{read}}$, provando que $k_{i+1} > k_i$ e nenhum registro é duplicado ou omitido.

### Pilar 5: Barreira de GC no VLog com Marcação Tricolor (`vlog_gc_barrier_kernel.rs`)
- **Modelo:** Conjuntos tricolores de identificadores de blob: Branco (candidato a descarte), Cinza (em análise) e Preto (vivo e inatingível pelo coletor).
- **Teorema de Barreira:** Qualquer blob alocado por transação ativa ou registrado em `VersionEdit` em staging é compulsoriamente promovido a Preto, provando $\text{SweptBlobs} \cap \text{ReachableBlobs} \equiv \emptyset$.

### Pilar 6: Terminação de Compactação por Métrica de Lyapunov (`compaction_lyapunov_kernel.rs`)
- **Modelo:** Função de potencial $E(\text{LSM}) = \sum_{l=0}^{L_{\max}} w_l \cdot \text{Debt}(l)$, onde $w_l$ pondera o custo de ordenação de cada nível.
- **Teorema:** Para qualquer estado fora do equilíbrio estável, cada passo de compactação legal reduz estritamente o potencial $E(\text{LSM}) - E(\text{LSM}') \ge 1$. Como $E(\text{LSM}) \ge 0$, o processo é bem-fundado e converge deterministicamente para o estado quiescente normal em passos finitos.

### Pilar 7: Não-Interferência e Zeroização Criptográfica (`zeroize_entropy_kernel.rs`)
- **Modelo:** Contrato de ciclo de vida de memória segura para buffers de criptografia e transações descartadas.
- **Teorema:** Ao término da transação ou purge de tabelas, o bloco de memória é submetido a zeroização forçada com barreira de compilador (`compiler_fence`), garantindo entropia residual zero e provando a não-interferência de fluxo de dados confidenciais.

### Pilar 8: Agendador de Tempo Denso Contínuo no DST (`dense_time_scheduler_kernel.rs`)
- **Modelo:** Espaço temporal real contínuo $\mathbb{R}_{\ge 0}$ discretizado em intervalos densos de perturbação $\epsilon \in (0, 10^{-6}]$.
- **Teorema:** Todo par de eventos concorrentes $(e_{\text{timer}}, e_{\text{io}})$ com diferença de tempo menor que $\delta$ é executado em ambas as permutações causais $(e_{\text{timer}} \prec e_{\text{io}})$ e $(e_{\text{io}} \prec e_{\text{timer}})$, provando ausência de pontos cegos temporais.

### Pilar 9: Confluência de 2PC sob Falha Concorrente Bipartida (`twopc_confluence_kernel.rs`)
- **Modelo:** Máquina de estados distribuída de transações cross-shard com falhas de coordenador e participantes em qualquer estágio intermediário.
- **Teorema:** Se qualquer nó decide `Commit`, todos os nós recuperados em qualquer ordem atingem o estado `Commit`. Se qualquer nó aborta antes da decisão mútua, todos convergem para `Abort`. A divergência é formalmente impossível ($\Pr[\text{Divergência}] = 0$).

### Pilar 10: Homomorfismo de Esquema e Preservação Semântica (`schema_homomorphism_kernel.rs`)
- **Modelo:** Morfismos entre esquemas versionados $S_1 \xrightarrow{\phi} S_2$ e decodificadores $D_1, D_2$.
- **Teorema:** Para qualquer valor $v$ do domínio de $S_1$, $D_2(E_1(v)) \equiv \phi(v)$. Não há mutação silenciosa de significado, truncamento espúrio ou corrupção de valores padrão durante upgrades de formato.

---

## 3. Esteira de Verificação Contínua (CVC Stage 16)

- Todos os 10 kernels implementados em Rust `#![forbid(unsafe_code)]`.
- Bateria de testes exaustivos e adversariais em `crates/pedradb-core/tests/rfc0282_dez_pilares_verificacao.rs`.
- Validação formal sob Miri (`-Zmiri-tree-borrows` e detecção de data-races).
- Inclusão do **Stage 16** em `scripts/verify_continuous_chain.sh`.
