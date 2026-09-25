# RFC-0285: Cinco Pilares Fundamentais de Verificação do Motor Puro PedraDB

- **Status:** Implementado & Verificado
- **Data:** 2026-09-25
- **Autores:** PedraDB Systems & Formal Verification Team
- **Escopo:** `pedradb-core`, `pedradb-spec`, Continuous Verification Chain (CVC)
- **Pré-requisitos:** RFC-0280, RFC-0281, RFC-0282, RFC-0283, RFC-0284, RFC-0270 (Zero-Twin), RFC-0273 (Absolute Rigor)

---

## 1. Contexto e Motivação

Com as fundações estabelecidas até o RFC-0284, o PedraDB alcançou uma cobertura sem precedentes em semântica de concorrência, recuperação de falhas e integridade lógica.

Contudo, ao isolar o escopo estritamente ao **motor puro mononó** (sem componentes distribuídos), cinco vetores estruturais críticos permaneciam como possíveis alvos de contestação formal e de sistemas de tempo real:
1. **Jitter e Não-Determinismo de Latência no Heap:** Chamadas dinâmicas ao alocador de memória do SO durante o pipeline quente de escrita (`put`), impedindo garantias de Worst-Case Execution Time (WCET).
2. **Condições de Corrida no Flush MemTable $\to$ SST:** Risco de chaves omitidas ou fora de ordem lexicográfica durante a transição concorrente `Active -> Immutable`.
3. **Reúso de Inodes e Bitrot de Diretório POSIX:** Vulnerabilidade ao abrir arquivos por nome onde o inode foi reciclado pelo filesystem ou apontado incorretamente por bitrot de diretório.
4. **Falhas Fatais `SIGBUS` sob Unmap Concorrente:** Leituras ativas de iteradores acessando páginas de memória virtual desmapeadas concorrentemente por compactações.
5. **Colisão de Chaves de Bloco em Cache por Encarnação:** Leituras de tabelas novas recebendo blocos antigos e desatualizados indexados apenas por `(file_number, offset)` após recuperação de crashes.

Este RFC resolve esses 5 itens integralmente com código de produção verificado, sem twins ou mocks (RFC-0270) e com `#![forbid(unsafe_code)]`.

---

## 2. Especificação dos Cinco Pilares

### Pilar 1: Contrato de Alocação Zero no Caminho Crítico (`hot_path_zero_alloc_kernel.rs`)
- **Problema:** Invocar `malloc` ou reallocar buffers durante `db.put()` causa spikes de latência imprevisíveis por contenção de locks de arena ou page-faults de THP.
- **Teorema 1 (Zero Alocação Dinâmica em Regime Permanente):**
  $$\forall \text{op} \in \text{SteadyStateWrites}: \Delta_{\text{HeapAlloc}}(\text{op}) \equiv 0 \text{ bytes}$$
- **Construção:** Implementação de `StaticSlabArena` e `ZeroAllocCommitPipeline`, onde slots de escrita, staging buffers de WAL e envelopes de commit operam sobre pools de memória estáticos com assertividade de zero alocações.

### Pilar 2: Bisimulação Concreta MemTable-SST (`memtable_flush_bisimulation_kernel.rs`)
- **Problema:** Se escritores mutarem a MemTable enquanto o iterador de flush varre para gravar o SST, a ordenação dos blocos pode quebrar ou chaves podem sumir.
- **Teorema 2 (Bijeção e Monotonicidade Estrita no Flush):**
  O corte de congelamento `Freeze` estabelece uma barreira de linearização que particiona as chaves. O iterador de flush satisfaz:
  $$\forall i < N-1: k_i <_{\text{lex}} k_{i+1} \quad \land \quad \text{Keys}(\text{SST}) \equiv \{k \in \text{MemTable} \mid \text{seq}(k) \le \text{FreezeSeq}\}$$
- **Construção:** `FrozenMemTableSnapshot` e `FlushSequenceValidator`, garantindo preservação unívoca de multiset e ordenação total estrita.

### Pilar 3: Identidade de Superbloco e Anti-Reúso de Inodes (`file_identity_superblock_kernel.rs`)
- **Problema:** O POSIX reutiliza números de inodes ao recriar arquivos; bitrot em diretórios pode fazer `open("000042.sst")` abrir um arquivo antigo ou incorreto.
- **Teorema 3 (Handshake de Superbloco Criptográfico):**
  Todo arquivo SST e WAL armazena um header de 64 bytes com `(Magic: [u8; 8], FileUUID: [u8; 16], FileNumber: u64, CreationEpoch: u64, SuperblockCrc: u32)`.
  $$\text{Open}(\text{Fd}) \implies \text{ReadSuperblock}(\text{Fd}) == \text{Manifest}(\text{ExpectedFile})$$
- **Construção:** `FileSuperblock`, `SuperblockHandshakeOracle`, falhando fechado antes de processar qualquer bloco se houver discrepância de identidade.

### Pilar 4: Barreira de Quiescência contra `SIGBUS` (`mmap_quiescence_barrier_kernel.rs`)
- **Problema:** Desmapear (`munmap`) ou truncar um arquivo SST deletado por compactação enquanto leitores ativos acessam páginas mapeadas causa falha fatal `SIGBUS`.
- **Teorema 4 (Quiescência de Páginas Mapeadas):**
  $$\text{PhysicalUnmap}(R) \implies \text{ActiveReaderLeases}(R) \equiv 0$$
- **Construção:** `MmapQuiescenceCoordinator` com contadores de referência por época (estilo RCU) e fila de descarte quiescente diferido, eliminando acessos a memória virtual órfã.

### Pilar 5: Desambiguação de Chaves de Cache por Encarnação (`block_cache_disambiguation_kernel.rs`)
- **Problema:** Indexar blocos de cache apenas por `(file_number, offset)` permite colisões silenciosas se números de arquivos coincidirem após restarts rápidos ou rollbacks.
- **Teorema 5 (Injetividade da Chave de Cache Tripartite):**
  A chave de cache $\text{CacheKey} = (\text{TableUUID}: [u8; 16], \text{BlockOffset}: u64, \text{ManifestGeneration}: u64)$ satisfaz:
  $$\text{CacheKey}(A) = \text{CacheKey}(B) \iff A \text{ e } B \text{ são o mesmíssimo bloco físico da mesma encarnação}.$$
- **Construção:** `DisambiguatedBlockCacheKey` e `CacheCollisionOracle`, garantindo isolamento total entre encarnações de arquivos.

---

## 3. Plano de Implementação

1. Implementar os 5 kernels em `crates/pedradb-core/src/`:
   - `hot_path_zero_alloc_kernel.rs`
   - `memtable_flush_bisimulation_kernel.rs`
   - `file_identity_superblock_kernel.rs`
   - `mmap_quiescence_barrier_kernel.rs`
   - `block_cache_disambiguation_kernel.rs`
2. Exportar todos os módulos em `crates/pedradb-core/src/lib_kernel.rs`.
3. Criar a suíte de testes `crates/pedradb-core/tests/rfc0285_cinco_pilares_motor_puro.rs`.
4. Integrar ao Miri gate em `scripts/miri_concurrency_gate.sh`.
5. Adicionar o Stage 19 em `scripts/verify_continuous_chain.sh`.
6. Validar a execução completa com 0 erros e 0 data-races.
