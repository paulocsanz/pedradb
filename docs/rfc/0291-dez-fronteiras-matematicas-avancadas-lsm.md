# RFC-0291: Dez Fronteiras Matemáticas e Estruturais Avançadas do LSM PedraDB

- **Status:** Ratificado / Em Implementação
- **Data:** 2026-09-28
- **Autor:** Antigravity / Equipe de Núcleo PedraDB
- **Área:** `pedradb-core` (Motor LSM de Nó Único Puro)
- **Políticas Cumpridas:** RFC-0270 (Zero-Twin), RFC-0273 (Rigor Absoluto), RFC-0041 (RocksDB Default Parity)

---

## 1. Sumário Executivo

A evolução do PedraDB atingiu um patamar formal sem precedentes através das RFCs 0270 a 0290, cobrindo 25 estágios contínuos de verificação matemática, concorrência sob memória fraca, simulação determinística de hardware e equivalência de bisimulação.

Contudo, sob a ótica de um matemático formalista e de um arquiteto adversarial de sistemas de armazenamento físico de alta integridade, existem 10 pontos de crítica profunda no comportamento assintótico, causal e físico do motor LSM:
1. **Estabilidade de Lyapunov no Write Stall:** Prevenção de ciclos limites caóticos e bifurcações oscilatórias na desaceleração de escritas.
2. **Homomorfismo de Prefix Seek sob Comparadores Customizados:** Garantia de que a projeção de prefixo comuta monotonicamente com a pré-ordem do comparador, eliminando perda silenciosa de chaves em varreduras por prefixo.
3. **Não-Interferência e Hazard Pointers em Snapshots de Longa Duração:** Preservação estrita de visibilidade contra descarte e `unlink` físico de SSTs sob GC concorrente.
4. **Fecho Monoidal e Associatividade do Colapso de Deltas do Manifest:** Prova algébrica de confluência do `VersionEdit` sob reescrita e compactação de log de catálogo.
5. **Refinamento Causal e Eliminação de Dangling Pointers no GC do vLog / Blob Storage:** Garantia transacional de two-phase safe purge entre ponteiros da LSM e arquivos físicos do vLog.
6. **Álgebra de Semianéis Não-Comutativos de Merge Operators:** Preservação de equivalência de resultado independentemente do instante de unificação (read-time vs. compaction-time).
7. **Cálculo de Redes e Conservação de Quota no I/O Rate Limiter:** Prevenção estrita de starvation e congelamento de flush sob saturação sustentada de escrita.
8. **Poset de Submissão e Desordenação entre Filas Paralelas NVMe:** Preservação de causalidade sob múltiplas Submission Queues e barreiras FUA/Flush.
9. **Integridade de Dois Estágios sob Compressão com Dicionário Compartilhado:** Eliminação de corrupção semântica silenciosa por descasamento de digest de dicionário ZSTD/LZ4.
10. **Refinamento de Confinamento de Falhas Físicas contra Bit-Rot e Torn-Reads em Background Scrubbing:** Isolamento cirúrgico de blocos corrompidos sem propagação catastrófica para o catálogo ou aborto indevido de compactações.

Esta RFC especifica as bases matemáticas e estruturais de cada uma das 10 fronteiras e institui os kernels de produção correspondentes em `crates/pedradb-core`.

---

## 2. Especificação Formal das Dez Fronteiras

### Fronteira 1: Estabilidade de Lyapunov no Write Stall
- **Axioma:** Seja $D_t \in \mathbb{R}_{\ge 0}$ a dívida normalizada de compactação no instante $t$ e $R_t \in (0, R_{\max}]$ a taxa de admissão de escrita concedida aos escritores.
- **Função Potencial de Lyapunov:**
  $$V(D_t, R_t) = \frac{1}{2} (D_t - D^*)^2 + \gamma \frac{1}{2} (R_t - R^*)^2$$
  onde $(D^*, R^*)$ é o ponto fixo de equilíbrio do motor.
- **Teorema de Convergência Monótona:**
  $$\forall t: D_t > D^* \implies \Delta V(t) = V(D_{t+1}, R_{t+1}) - V(D_t, R_t) \le -\kappa \|(D_t - D^*, R_t - R^*)\|^2$$
  com taxa de atenuação $\kappa > 0$, provando a ausência de órbitas periódicas e chattering.

### Fronteira 2: Homomorfismo de Prefix Seek sob Comparadores Customizados
- **Definição:** Seja $(\mathcal{K}, \prec)$ a ordem total do comparador de chaves e $\pi: \mathcal{K} \to \mathcal{P}$ a função de extração de prefixo.
- **Condição Homomórfica de Prefixo:**
  $$\forall k_1, k_2 \in \mathcal{K}: \pi(k_1) \prec_\pi \pi(k_2) \implies k_1 \prec k_2$$
- **Invariante de Varredura Confluente:** Se $\pi(k_1) = \pi(k_2)$, então qualquer chave $k_3$ tal que $k_1 \prec k_3 \prec k_2$ necessariamente satisfaz $\pi(k_3) = \pi(k_1)$. Qualquer comparador que viole esta propriedade é rejeitado na inicialização do índice.

### Fronteira 3: Não-Interferência e Hazard Pointers em Snapshots de Longa Duração
- **Definição:** Seja $\mathcal{S}_{\text{live}}$ o conjunto de snapshots ativos com suas respectivas sequências $s.\text{seq}$. Seja $\mathcal{F}_{\text{disk}}$ o conjunto de arquivos SST registrados no catálogo e $\mathcal{P}_{\text{hazard}}$ o conjunto de arquivos pinados por leitores.
- **Invariante de Separação:**
  $$\forall F \in \mathcal{F}_{\text{disk}}: (\exists s \in \mathcal{S}_{\text{live}}: F.\text{min\_seq} \le s.\text{seq} \le F.\text{max\_seq}) \implies F \in \mathcal{P}_{\text{hazard}} \implies \text{Unlink}(F) = \text{FORBIDDEN}$$
- Prova que nenhum arquivo de dados é descartado enquanto acessível por qualquer snapshot vivo.

### Fronteira 4: Fecho Monoidal do Colapso de Deltas do Manifest
- **Álgebra de Versões:** Seja $(\mathcal{V}, \star, \mathcal{V}_0)$ o monóide onde $\mathcal{V}$ é o catálogo de níveis e $\star$ é a aplicação de deltas $\delta \in \text{VersionEdit}$.
- **Associatividade Estrita:**
  $$(\mathcal{V} \star \delta_1) \star \delta_2 = \mathcal{V} \star (\delta_1 \star \delta_2)$$
- **Idempotência de Checkpoint:**
  $$\text{Collapse}(\delta_1 \star \dots \star \delta_n) \equiv \mathcal{V}_{\text{compact}} \iff \forall k: \text{Lookup}(\mathcal{V}_n, k) = \text{Lookup}(\mathcal{V}_{\text{compact}}, k)$$

### Fronteira 5: Refinamento Causal de vLog / Blob Storage GC
- **Refinamento FSCQ-Class:** Seja $P = (f_{\text{vlog}}, \text{offset}, \text{len})$ um ponteiro de blob indexado na LSM.
- **Protocolo Two-Phase Safe Purge:**
  1. *Fase 1 (Reescrita Atômica):* O GC grava os registros ativos no novo vLog $f_{\text{new}}$ e obtém novos ponteiros $P'$.
  2. *Fase 2 (Commit LSM):* A substituição de $P \mapsto P'$ é comitada no `MANIFEST` via barreira atômica.
  3. *Fase 3 (Descarte Seguro):* O vLog antigo $f_{\text{old}}$ é desvinculado **apenas e somente após** a persistência da versão LSM sem referências a $f_{\text{old}}$.

### Fronteira 6: Álgebra de Semianéis Não-Comutativos de Merge Operators
- **Estrutura:** Seja $\mathcal{M}$ o semianel de operandos de merge com adição coalescente $\oplus$ e composição sequencial $\circ$.
- **Axiomas de Aniquilador:**
  $$\text{Put}(v) \circ \text{Merge}(m) = \text{Put}(v \oplus m)$$
  $$\text{Delete} \circ \text{Merge}(m) = \text{Put}(\text{Identity} \oplus m)$$
- **Associatividade:**
  $$(m_1 \oplus m_2) \oplus m_3 = m_1 \oplus (m_2 \oplus m_3)$$
  garantindo confluência idêntica entre leituras e compactações.

### Fronteira 7: Cálculo de Redes e Conservação de Quota no Rate Limiter
- **Envelope de Tráfego:** Escritores admitem rajadas delimitadas pelo envelope $(\sigma, \rho)$.
- **Lei de Conservação do Flush:**
  $$\mu_{\text{flush}}(t) = \max\left(\mu_{\text{min\_configured}}, \, \alpha \cdot \frac{\text{DirtyMemBytes}(t)}{\text{MemBudget}} \cdot \text{DiskCapacity}\right)$$
  provando que sob saturação $(\text{DirtyMemBytes} \to \text{MemBudget})$, $\mu_{\text{flush}} \to \text{DiskCapacity}$, impedindo starvation de flush.

### Fronteira 8: Poset de Submissão e Desordenação entre Filas NVMe
- **Poset:** Sejam $I_1 = (F_1, B_1)$ e $I_2 = (F_2, B_2)$ operações de escrita despachadas em Submission Queues $SQ_a \ne SQ_b$.
- **Invariante Causal:** Se $I_1$ é pré-requisito de visibilidade de $I_2$ (ex: SST antes do bloco de catálogo correspondente), o escalonador deve emitir uma barreira física de hardware (`FUA` ou `NVMe Flush Barrier`) antes de submeter $I_2$, preservando o corte consistente sob interrupção de energia.

### Fronteira 9: Integridade de Dois Estágios sob Compressão com Dicionário
- **Integridade Criptográfica:** Cada bloco comprimido $C$ com dicionário $D$ carrega no seu cabeçalho:
  $$H_{\text{phys}} = \text{CRC32c}(C), \quad H_{\text{dict}} = \text{BLAKE3}(D)[0..8], \quad H_{\text{payload}} = \text{CRC32c}(\text{Decompress}(C, D))$$
- **Rejeição Estrita:** Se $H_{\text{dict}}$ do bloco não coincidir exatamente com o digest do dicionário ativo, a descompressão é abortada imediatamente com `DictionaryMismatch`, impedindo descompressão em lixo semântico.

### Fronteira 10: Refinamento de Confinamento de Falhas Físicas contra Bit-Rot
- **Confinamento:** Dado um bloco corrompido $B_{\text{corrupt}}$ com chave mínima $k_a$ e máxima $k_b$:
  $$\forall k \notin [k_a, k_b]: \text{Read}(k) = \text{UncompromisedValue}(k)$$
  $$\forall S \in \text{OrthogonalSSTs}: \text{Compact}(S) = \text{Success}$$
- O erro é estritamente isolado no intervalo do bloco, gerando relatório de auditoria e preservando a compactação do restante da base.

---

## 3. Matriz de Módulos e Kernels de Produção

| Fronteira | Arquivo Kernel | Responsabilidade Formal |
|---|---|---|
| F1 | `crates/pedradb-core/src/lyapunov_write_stall_kernel.rs` | Estabilidade assintótica de Lyapunov no write stall |
| F2 | `crates/pedradb-core/src/prefix_seek_homomorphism_kernel.rs` | Verificação do homomorfismo prefix seek & comparador |
| F3 | `crates/pedradb-core/src/snapshot_gc_hazard_pointer_kernel.rs` | Não-interferência de snapshots de longa duração e hazard pointers |
| F4 | `crates/pedradb-core/src/manifest_delta_collapse_kernel.rs` | Fecho monoidal e colapso de deltas do Manifest |
| F5 | `crates/pedradb-core/src/vlog_blob_gc_refinement_kernel.rs` | Protocolo Two-Phase Safe Purge para vLog / Blob GC |
| F6 | `crates/pedradb-core/src/merge_operator_semiring_kernel.rs` | Semianéis não-comutativos com absorção para Merge Operators |
| F7 | `crates/pedradb-core/src/io_rate_limiter_conservation_kernel.rs` | Conservação e auto-expansão de quota no I/O Rate Limiter |
| F8 | `crates/pedradb-core/src/nvme_queue_poset_reorder_kernel.rs` | Poset de reordenação em multi-queues NVMe e barreiras FUA |
| F9 | `crates/pedradb-core/src/two_stage_compression_dictionary_kernel.rs` | Integridade física e semântica com dicionário compartilhado |
| F10 | `crates/pedradb-core/src/fault_isolation_bitrot_kernel.rs` | Confinamento e isolamento estrito de falhas físicas de bit-rot |

---

## 4. Conclusão e Critérios de Aceite

1. Todos os 10 kernels implementados em Rust puro com `#![forbid(unsafe_code)]`.
2. Suíte unitária `rfc0291_dez_fronteiras_matematicas_lsm.rs` passando 100% verde.
3. Execução sob o simulador Miri com `-Zmiri-tree-borrows` sem data races nem UB.
4. Registro na Continuous Verification Chain (`scripts/verify_continuous_chain.sh` como Estágio 26).
5. Documentação histórica atualizada em `docs/TRAJETORIA.md`.
