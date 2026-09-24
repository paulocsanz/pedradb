# RFC-0279: Confluência do MANIFEST (Church-Rosser LSM), Liveness e Não-Inanição, Serializabilidade SSI e Verificação de Cola Causal

- **Status:** Proposto e Implementado
- **Data:** 2026-09-24
- **Autores:** PedraDB Formal Verification Core Team
- **Objetivo:** Resolver de forma definitiva os 5 pontos fracos teóricos avançados identificados na auditoria matemática: (1) Confluência e comutatividade do MANIFEST sob concorrência; (2) Garantia formal de Liveness e ausência de inanição (*starvation-freedom*) no group commit e backpressure; (3) Prevenção de anomalias de Write-Skew via Serializabilidade SSI; (4) Determinismo e associatividade de operadores de Merge; (5) Contratos de causalidade estrita na cola imperativa de handlers.

---

## 1. Contexto e Motivação Teórica

A auditoria matemática identificou que sistemas de armazenamento verificados frequentemente falham não na aritmética local, mas nas propriedades de sistema dinâmicas:
1. **Confluência de VersionEdit (MANIFEST):** Quando múltiplos flushes e compactações concorrentes comitam deltas de arquivo no MANIFEST, a ordem de chegada no disco não pode criar estados divergentes ou arquivos órfãos. É necessária a prova da Propriedade do Diamante (Church-Rosser).
2. **Liveness e Não-Inanição:** Teoremas clássicos cobrem apenas *Safety* (\(\square \neg \text{Bad}\)). É indispensável provar *Liveness* (\(\square \lozenge \text{Good}\)): que todo escritor eventualmente comita em tempo limitado e que o backpressure não gera livelock ou deadlock circular de recursos.
3. **Isolamento e Write-Skew (SSI):** Snapshot Isolation permite anomalias de Write-Skew. Uma camada de detecção de anti-dependências de leitura/escrita (*rw-antidependencies*) é necessária para prover Serializabilidade Estrita.
4. **Determinismo de Merge Operators:** Operadores de fusão associativa devem provar comutatividade/associatividade para garantir que leituras dinâmicas na MemTable coincidam exatamente com blocos fundidos em background na compactação.
5. **Ordenação Causal na Cola:** Os 128k LOC de handlers imperativos devem ser guardados por uma máquina de estados de causalidade estrita que impeça a emissão de Ack sem barreira durável prévia.

---

## 2. P0: Fundações Estruturais

### P0.1: Álgebra de Confluência do MANIFEST (`manifest_confluence_kernel.rs`)
- **Estado do VersionSet:** Mapeamento de níveis \(L_0 \dots L_6\) com conjuntos ordenados de metadados de SST (\(\text{file\_num}, [k_{\min}, k_{\max}], \text{seq}_{\max}\)).
- **Delta de Versão (\(\Delta\)):** Par \((\text{deleted\_files}, \text{added\_files})\).
- **Teorema de Confluência Local (Propriedade do Diamante):**
  Para quaisquer duas edições de versão independentes \(\Delta_1\) e \(\Delta_2\) tais que seus conjuntos de arquivos modificados sejam disjuntos:
  $$(V \oplus \Delta_1) \oplus \Delta_2 \equiv (V \oplus \Delta_2) \oplus \Delta_1$$
- **Invariante de Fechamento de Inventário:** Zero arquivos órfãos em disco e zero ponteiros pendentes para arquivos inexistentes.

### P0.2: Liveness e Starvation-Freedom no Group Commit (`liveness_progress_kernel.rs`)
- **Função de Potencial \(\Phi(\sigma)\):** Define a distância de cada thread escritora até o commit.
- **Teorema de Progresso Estrito:** A cada ciclo de flush/fsync, \(\Phi(\sigma') < \Phi(\sigma)\), garantindo que todo escritor na fila seja atendido em no máximo \(W \le \text{MAX\_GROUP\_EPOCHS}\) ciclos.
- **Imunidade a Deadlock no Backpressure:** Grafo de dependência de recursos estritamente acíclico entre buffers de memória da MemTable e créditos de cota de I/O em disco.

---

## 3. P1: Consistência e Semântica de Dados

### P1.1: Detecção de Write-Skew e Serializabilidade Estrita SSI (`ssi_conflict_kernel.rs`)
- Rastreador de dependências \(rw\) (*SIREAD locks* / pivôs de conflito).
- Detecção em tempo linear de ciclos no Grafo de Serialização Multi-Versão (MSR):
  $$T_1 \xrightarrow{rw} T_2 \xrightarrow{rw} T_1 \implies \text{Abort}(T_2)$$
- Eliminação total de Write-Skew, promovendo Snapshot Isolation a Serializable Snapshot Isolation.

### P1.2: Determinismo e Associatividade do Operador de Merge (`merge_determinism_kernel.rs`)
- Contrato formal de associatividade:
  $$\forall a, b, c: \quad \text{Merge}(\text{Merge}(a, b), c) \equiv \text{Merge}(a, \text{Merge}(b, c))$$
- Prova de equivalência: A visualização obtida via `MergeIterator` sobre MemTable + L0 é idêntica à saída do `CompactKernel` quando este materializa os mesmos operandos.

---

## 4. P2: Integridade de Seam e Execução Contínua

### P2.1: Contratos Causais de Ordem na Cola de Handlers (`causal_seam_kernel.rs`)
- Máquina de estados causal: transições obrigatórias:
  $$\text{Unstaged} \to \text{Staged} \to \text{Fsynced} \to \text{ManifestCommitted} \to \text{ClientAcked}$$
- Interceptador de handlers: aborta imediatamente qualquer tentativa de resposta a cliente sem o registro prévio do token de persistência física.

### P2.2: Suíte Integrada e Continuous Verification Chain
- Testes exaustivos em `crates/pedradb-core/tests/rfc0279_confluence_liveness_ssi.rs`.
- Inclusão dos estágios 13, 14 e 15 em `scripts/verify_continuous_chain.sh`.
