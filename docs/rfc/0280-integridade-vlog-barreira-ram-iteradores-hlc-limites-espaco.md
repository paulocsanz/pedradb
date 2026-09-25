# RFC-0280: Integridade Referencial do VLog, Barreira de Memória Pré-Flush, Linearizabilidade de Iteradores sob Compactação, Relógio Híbrido Monótono e Limites de Espaço Amortizado

- **Status:** Proposto e Implementado
- **Data:** 2026-09-25
- **Autores:** PedraDB Formal Verification Core Team
- **Objetivo:** Resolver de forma integral e irrefutável as 5 críticas teóricas e fraquezas de sistemas apontadas na auditoria matemática avançada:
  1. Integridade referencial do VLog em separação de chave-valor (ausência total de dangling blob pointers pós-GC ou crash);
  2. Barreira de invariante de ordenação pré-flush e proteção contra bit-rot em RAM;
  3. Linearizabilidade e monotonicidade de iteradores longos com pinning de arquivos SST sob compactação concorrente;
  4. Horizonte lógico monótono de tempo (HLC) imune a saltos retroativos de relógio físico (NTP time-travel);
  5. Limites matemáticos amortizados de amplificação de espaço temporário e prevenção de estol de escrita por `ENOSPC`.

---

## 1. Contexto e Motivação Matemática

Apesar da eliminação dos problemas clássicos de banco de dados nos RFCs anteriores, uma inspeção teórica rigorosa aponta cinco vulnerabilidades em sistemas de arquivos e memória volátil:
1. **Dangling Pointers em KV Separation:** Sistemas que separam valores grandes (WiscKey / BlobDB) frequentemente sofrem corrupção silenciosa quando ponteiros no LSM apontam para offsets inválidos de VLog após Garbage Collection ou reinício pós-queda de energia.
2. **Corrupção Silenciosa em DRAM:** Assumir que a memória volátil nunca sofre bit-flips é ingênuo. Se uma skiplist for corrompida em RAM, uma SSTable com chaves fora de ordem é gerada, corrompendo a busca binária de forma irreversível no disco.
3. **Invalidação de Iteradores por Unlink de Arquivo:** Range scans longos podem ler SSTables antigas que são deletadas pelo SO enquanto o leitor percorre o conjunto de dados. É necessária prova de retenção de handles e monotonicidade de passos.
4. **Violação de TTL por Ajustes de Relógio:** Se o relógio do sistema retroceder por sincronização de NTP ou avanço de leap second, políticas de expiração de dados (TTL) quebram. É indispensável um relógio monótono de horizonte lógico híbrido.
5. **Cascatas de Compactação e Esgotamento de Disco:** Compactações simultâneas exigem espaço temporário duplicado para escrever novos arquivos antes de desvincular os antigos. É necessário um teto amortizado provando a ausência de `ENOSPC`.

---

## 2. P0: Integridade do Armazenamento de Dados

### P0.1: Integridade Referencial e Validade do VLog (`vlog_integrity_kernel.rs`)
- Predicado formal de validade:
  $$\forall (k, \text{vlog\_ptr}) \in \text{LsmTree}: \quad \text{vlog\_ptr.offset} + \text{vlog\_ptr.len} \le \text{VlogFileSize} \land \text{CRC}(\text{Payload}) == \text{ExpectedCRC}$$
- Teorema de Troca Atômica do GC: Durante a migração de valores de `VALUES.vlog` para `VALUES.vlog.new`, o swap atômico no manifesto garante que em **nenhum instante de tempo observável** existe um ponteiro ativo apontando para um offset não persistido.

### P0.2: Barreira de Ordenação Pré-Flush e Proteção contra Bit-Rot (`ram_preflush_barrier_kernel.rs`)
- Barreira indutiva de pré-serialização:
  $$\forall i < j: \quad \text{Key}_i <_{\text{cmp}} \text{Key}_j \land (\text{Key}_i == \text{Key}_j \implies \text{Seq}_i > \text{Seq}_j)$$
- Fail-Closed: Qualquer inversão de ordenação causada por bit-rot ou corrida em memória na MemTable é interceptada **antes** da codificação do bloco SST, impedindo a contaminação irreversível do armazenamento físico.

---

## 3. P1: Robustez de Execução e Tempo Físico

### P1.1: Linearizabilidade e Retenção de Iteradores Vivos (`iterator_pinning_kernel.rs`)
- Invariante de Pinning de Arquivos (*Hazard Reference Counting*): Arquivos SST envolvidos em compactação só podem sofrer `unlink` físico quando o contador de iteradores ativos referenciando a versão chegar a zero.
- Teorema de Monotonicidade de Passo:
  $$\text{Iter.next}() = (k_{\text{next}}, v_{\text{next}}) \implies k_{\text{next}} > k_{\text{prev}} \land \text{seq}(k_{\text{next}}) \le \text{SnapshotSeq}$$

### P1.2: Horizonte Lógico Monótono de Tempo (HLC) (`monotonic_clock_kernel.rs`)
- Função de avanço do relógio lógico:
  $$H_{\text{now}} = \max(\text{PhysicalWallClock}, H_{\text{prev}} + 1)$$
- Teorema de Imunidade a Time-Travel: Saltos para trás do relógio do sistema operacional (NTP skew) têm impacto zero sobre as decisões de expiração de dados e expiração de tombstones.

---

## 4. P2: Limites Amortizados de Espaço

### P2.1: Limites Amortizados de Espaço Temporário (`space_amplification_kernel.rs`)
- Formalização do teto de espaço:
  $$\text{TransientCompactionSpace} \le \sum_{f \in \text{Inputs}} \text{Size}(f) \le \beta \cdot \text{TotalCapacity}$$
- Bloqueio preventivo: Se o espaço livre em disco for menor que a estimativa da maior compactação em curso, a compactação é adiada para evitar pane de `ENOSPC`.

---

## 5. Verificação e Continuous Verification Chain

- Testes exaustivos em `crates/pedradb-core/tests/rfc0280_vlog_ram_iter_hlc_space.rs`.
- Verificação Miri com Tree Borrows e Data Race detector.
- Inclusão do Stage 14 na esteira de verificação contínua (`scripts/verify_continuous_chain.sh`).
