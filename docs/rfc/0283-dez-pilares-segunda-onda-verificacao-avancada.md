# RFC-0283: Os Dez Pilares da Segunda Onda de Verificação Avançada de Sistemas de Armazenamento

- **Status:** Proposto e Implementado
- **Data:** 2026-09-25
- **Autores:** PedraDB Formal Verification Core Team
- **Objetivo:** Resolver de forma integral, matemática e irrefutável os 10 novos pontos críticos e vulnerabilidades teóricas profundas identificadas na auditoria avançada:
  1. **Ausência de Inanição e Limite Estrito de Ultrapassagem ($K$-Exclusion / Bounded Overtaking):** Prova de wait-freedom delimitado no grupo de commit de escrita;
  2. **Teorema da Inversão Bijetiva de Codecs de Compressão:** Prova de que descompressão é um inverso perfeito da compressão ($\forall x: \mathcal{D}(\mathcal{C}(x)) \equiv x$);
  3. **Soundness de Delta-Encoding por Prefixo e Pontos de Reinício em Blocos SST:** Indução formal provando reconstrução incremental exata de chaves sem riscos de estouro de buffer;
  4. **Isolamento e Recuperação Atômica Multi-Column Family (Cross-CF Replay Independence):** Particionamento rigoroso de corte por CF no WAL, eliminando contaminação cruzada;
  5. **Imunidade a Envenenamento de Cache em DRAM (In-Memory Block Cache Bit-Flip Protection):** Sentinelas de integridade e canários de página protegendo blocos descompactados no Block Cache;
  6. **Detecção Dinâmica de Ciclos de Anti-Dependência em SSI (Full Serialization Graph SGC):** Detector dinâmico de estruturas perigosas ($rw \to rw$) garantindo serializabilidade estrita;
  7. **Prevenção de Esgotamento de Espaço por Snapshots Abandonados (Snapshot Epoch Lease):** Contrato de leases de época com revogação segura *fail-closed* contra vazamentos de espaço;
  8. **Imunidade a Inversão de Prioridade no Caminho Crítico de Commit:** Desacoplamento estrito de filas e tetos de prioridade entre I/O de cliente e I/O de manutenção (flush/compaction/GC);
  9. **Persistência Estrita do Diretório-Pai POSIX (`sync_dir` contra Inodes Órfãos):** Prova de ordenação atômica onde o MANIFEST nunca publica arquivos sem confirmação prévia do diretório-pai;
  10. **Continuidade Monotônica na Transição Snapshot-para-Log na Replicação:** Colagem hermética de fronteira de réplica entre imagem de estado SST e stream ao vivo do WAL.

---

## 1. Contexto e Fundamentos Matemáticos

A solidez de um motor de armazenamento que visa o padrão **seL4 / FSCQ / VeriBetrKV** exige que todas as arestas periféricas do sistema sejam matematicamente demonstradas:
- O progresso concorrente não pode depender de suposições de agendamento benevolente do SO;
- A compressão e descompressão de blocos deve ser uma bijeção provada sobre $\Sigma^*$;
- A reconstrução de chaves ordenadas com prefixos compartilhados deve satisfazer invariantes indutivos invioláveis;
- A concorrência entre diferentes famílias de colunas (Column Families) deve manter isolamento de falhas e de recuperação;
- A memória volátil que armazena blocos em cache deve ter validação de integridade contra corrupção silenciosa em RAM;
- Transações serializáveis (SSI) devem impedir ciclos arbitrários de dependências de leitura/escrita ($rw \to rw$);
- Snapshots mantidos por clientes devem ter horizontes de vida finitos para garantir a cota amortizada de espaço em disco;
- O caminho crítico de escrita não pode sofrer inversão de prioridade diante de tarefas volumosas de manutenção;
- O sistema de arquivos POSIX exige sincronização de metadados do diretório-pai para evitar que inodes válidos fiquem inacessíveis pós-queda;
- A replicação distribuída requer que a passagem de bastão entre um snapshot inicial e o stream de log contínuo seja isomórfica a um log contínuo sem lacunas nem duplicações.

---

## 2. Especificação Formal dos 10 Pilares (Segunda Onda)

### Pilar 1: Bounded Overtaking no Group Commit (`starvation_freedom_kernel.rs`)
- **Teorema de $K$-Exclusão:** Seja $T_{\text{enq}}$ o momento em que uma thread $w$ entra na fila de commit. O número de threads admitidas para commit antes de $w$ é estritamente limitado por $K$:
  $$|\{ w' \mid w' \text{ admitido antes de } w \land w' \text{ enfileirado após } w \}| \le K$$
- Garante ausência total de inanição (*starvation-freedom*) sem depender de escalonador justo do SO.

### Pilar 2: Inversão Bijetiva de Codecs (`codec_inversion_kernel.rs`)
- **Teorema da Identidade:** Para qualquer buffer de entrada $x \in \Sigma^*$:
  $$\mathcal{D}(\mathcal{C}(x)) \equiv x$$
- Além disso, qualquer buffer truncado ou com tokens corrompidos retorna `Err(CodecError::CorruptStream)` em modo estritamente *fail-closed*, sem provocar *out-of-bounds read*.

### Pilar 3: Integridade de Delta-Encoding por Prefixo (`prefix_delta_restart_kernel.rs`)
- **Invariante Indutivo de Bloco:** Seja $(k_0, \dots, k_{n-1})$ uma sequência de chaves ordenadas. O bloco codificado particiona a sequência em intervalos $[R_i, R_{i+1})$. A função de reconstrução incremental satisfaz:
  $$\text{Reconstruct}(R_i, j) \equiv k_{R_i + j} \quad \forall 0 \le j < \text{RestartInterval}$$
- Prova de que `shared_len <= prev_key.len()` e `shared_len + unshared_len <= MAX_KEY_SIZE`.

### Pilar 4: Isolamento e Recuperação Multi-CF (`cross_cf_isolation_kernel.rs`)
- **Teorema de Particionamento de Replay:** Dado um fluxo de WAL misto contendo registros de famílias de colunas $CF_1, \dots, CF_m$:
  $$\text{Replay}(\text{WAL}, CF_i, \text{Cutoff}_i) \equiv \text{Replay}(\pi_{CF_i}(\text{WAL}), \text{Cutoff}_i)$$
- Mutações pertencentes a uma CF já persistida são descartadas sem afetar a recuperação de CFs com dados pendentes.

### Pilar 5: Sentinela de Integridade no Cache de Blocos (`cache_canary_sentinel_kernel.rs`)
- **Contrato de Canário em RAM:** Cada bloco armazenado no cache volátil é envolvido por canários de borda e um checksum em memória calculado no momento da admissão:
  $$\text{CheckRAM}(B) \implies \text{CRC32C}(B.\text{payload}) == B.\text{admitted\_crc} \land B.\text{canary\_head} == B.\text{canary\_tail}$$
- Qualquer bit-flip em DRAM ou escrita desgovernada por ponteiro é interceptado antes da entrega ao leitor.

### Pilar 6: Grafo Dinâmico de Serialização SSI (`ssi_cycle_detector_kernel.rs`)
- **Teorema de Fekete/Cahill:** Uma execução concorrente sob Snapshot Isolation viola a Serializabilidade estrita se e somente se o grafo de dependências contém um ciclo com arestas consecutivas de anti-dependência de leitura-escrita ($rw \to rw$).
- O kernel rastreia ativamente as arestas `in_conflict` e `out_conflict`, forçando o aborto da transação pivô assim que uma estrutura perigosa é fechada.

### Pilar 7: Leases de Época para Snapshots (`snapshot_epoch_lease_kernel.rs`)
- **Contrato de Tempo de Retenção Delimitado:** Todo snapshot alocado possui uma cota de épocas $\tau$. Se o relógio lógico da árvore atingir $\text{current\_epoch} > \text{snapshot\_epoch} + \tau$, o snapshot é considerado expirado:
  $$\text{IsRevoked}(S) \implies \text{PurgeOracle::AllowTombstoneRemoval}(S.\text{seq})$$
- Leituras posteriores naquele snapshot retornam `Err(SnapshotExpired)`, impedindo que clientes lentos travem a liberação de espaço em disco.

### Pilar 8: Imunidade a Inversão de Prioridade no Commit (`priority_inversion_freedom_kernel.rs`)
- **Teorema de Teto de Prioridade:** Filas de admissão de clientes (alta prioridade) são desacopladas das tarefas de manutenção em lote (baixa prioridade: flush, compaction, GC).
- Nenhuma thread de cliente espera por um lock detido por uma tarefa de manutenção que não execute com herança temporária de prioridade limitada a $O(1)$ passos.

### Pilar 9: Sincronização Estrita do Diretório-Pai POSIX (`posix_dir_sync_kernel.rs`)
- **Tríade Atômica de Persistência:** Para qualquer arquivo novo $F$ criado no diretório $D$:
  $$\text{State}(F) = \text{Durable} \iff \text{fdatasync}(F) \prec \text{fsync}(D) \prec \text{ManifestAppend}(F)$$
- Elimina matematicamente a possibilidade de inodes órfãos decorrentes de falhas de energia antes do commit do journal de diretório do sistema de arquivos.

### Pilar 10: Continuidade Monotônica Snapshot-para-Log na Replicação (`replication_catchup_boundary_kernel.rs`)
- **Teorema de Colagem Hermética:** Seja $S_{\max}$ o maior número de sequência contido no snapshot de réplica. O primeiro registro do log WAL transmitido deve ter sequência $S_{\text{first}} = S_{\max} + 1$:
  $$S_{\text{first}} = S_{\max} + 1 \implies \text{Gap} = \emptyset \land \text{Overlap} = \emptyset$$
- Garante continuidade linear exata sem necessidade de deduplicação redundante ou risco de mutações perdidas.

---

## 3. Esteira de Verificação Contínua (CVC Stage 17)

- Todos os 10 kernels implementados em Rust `#![forbid(unsafe_code)]`.
- Testes unitários e de estresse em `crates/pedradb-core/tests/rfc0283_dez_pilares_segunda_onda.rs`.
- Verificação de concorrência com Miri (`-Zmiri-tree-borrows` e detecção de data-race).
- Inclusão do **Stage 17** em `scripts/verify_continuous_chain.sh`.
