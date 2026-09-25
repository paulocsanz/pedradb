# RFC-0281: Zero Falsos Negativos do Bloom Filter, Recuperação Dual WAL-MANIFEST, Soundness de Purga de Tombstones, Axioma de Transitividade Estrita do Comparador e Contrato de Direct I/O

- **Status:** Proposto e Implementado
- **Data:** 2026-09-25
- **Autores:** PedraDB Formal Verification Core Team
- **Objetivo:** Resolver de forma integral, matemática e irrefutável os 5 pontos frágeis e vulnerabilidades teóricas apontadas na auditoria matemática avançada:
  1. **Zero Falsos Negativos no Bloom Filter:** Prova formal e oráculo axiomático de que o esquema Kirsch-Mitzenmacher nunca produz falsos negativos ($\forall k \in \text{InsertedKeys}: \text{may\_contain}(k) == \text{true}$), com monotonicidade estrita do vetor de bits;
  2. **Invariante de Recuperação Dual WAL-MANIFEST:** Eliminação do risco de ressurreição de registros zumbi (*Zombie Record Resurrection*) através do barramento formal acoplado `min_log_number` e `earliest_readable_seq`;
  3. **Soundness de Purga de Tombstones:** Oráculo de eliminação segura de tombstones provando que deleções só podem ser descartadas quando não houver snapshots vivos e nenhuma chave sombreada em níveis inferiores ($L+1 \dots L_{\max}$);
  4. **Axioma de Transitividade Estrita do Comparador de Chaves:** Verificador axiomático provando que o comparador de chaves satisfaz a álgebra de *Strict Weak Ordering* (irreflexividade, assimetria, transitividade de $<$ e transitividade de equivalência $\sim$) sob qualquer payload binário com NUL bytes arbitrários;
  5. **Contrato de Alinhamento e Coerência de Direct I/O:** Verificador e alocador de alinhamento modular estrito a 4096 bytes em ponteiros de memória, offsets de arquivo e comprimentos de transferência para operações `O_DIRECT` e DMA de hardware.

---

## 1. Contexto e Motivação Matemática

Apesar das garantias de linearizabilidade, bisimulação indutiva e imunidade a falhas de hardware já formalizadas, uma análise matemática profunda dos fundamentos de estruturas LSM-tree expõe cinco potenciais fragilidades teóricas:

1. **Risco de Falsos Negativos em Filtros de Bloom:** Em motores LSM, falsos positivos aumentam I/O redundante, mas falsos negativos destroem a consistência do banco de dados (o motor afirma que uma chave não existe quando ela está gravada no SSTable). Qualquer falha em cálculo de hash duplo Kirsch-Mitzenmacher, truncamento inteiro de 64 para 32 bits, ou ordenação de bits pode causar falso negativo silencioso.
2. **Ressurreição de Chaves Zumbi (Dual-Log Inconsistency):** O estado de um LSM reside simultaneamente no log de metadados (`MANIFEST`) e nos logs de escrita imediata (`WAL`). Se o replay do WAL após crash não respeitar a fronteira de corte do MANIFEST (`earliest_readable_seq` e `min_log_number`), operações antigas já sobregravadas ou deletadas podem ser reaplicadas sobre o estado novo de SSTables, ressuscitando chaves mortas.
3. **Desenmascaramento Indevido de Tombstones (Tombstone Unmasking):** A compactação de um nível $L$ para $L+1$ remove tombstones para recuperar espaço. Contudo, se um tombstone for descartado enquanto ainda existe uma versão antiga da mesma chave em $L+2 \dots L_{\max}$ (ou um snapshot ativo enxergando a versão anterior), a versão antiga é subitamente "desenmascarada", corrompendo a semântica de deleção.
4. **Violação de Strict Weak Ordering em Comparadores:** A busca binária em blocos de dados, a integridade da MemTable e o merge de iteradores assumem que o comparador define uma relação de ordem fraca estrita. Se bytes nulos intermediários (`\0`), desvios de sinal de byte ou colisões parciais quebrarem a transitividade ($a < b \land b < c \implies a < c$) ou a equivalência, o motor entra em loop infinito ou perde chaves em lookups pontuais.
5. **Falhas de Alinhamento em Direct I/O (`O_DIRECT`):** Quando bypassamos o page-cache do kernel para ler SSTs ou gravar WAL com DMA direta, drivers NVMe e o kernel Linux/POSIX exigem que endereço de memória, offset no arquivo e tamanho do buffer sejam múltiplos de 4096 bytes. Uma violação resulta em erros silenciosos ou `EINVAL` catastrófico em tempo de execução.

---

## 2. P0: Invariantes Críticos de Armazenamento e Recuperação

### P0.1: Zero Falsos Negativos do Bloom Filter (`bloom_soundness_kernel.rs`)
- **Teorema de Monotonicidade do Vetor de Bits:**
  Seja $B_t$ o vetor de bits após $t$ inserções. A operação de inserção para uma chave $k$ computa o conjunto determinístico de sondas $I(k) = \{ g_i(k) \pmod m \mid 0 \le i < k_{\text{probes}} \}$ e aplica bitwise OR:
  $$B_{t+1} = B_t \cup I(k)$$
  Portanto, $B_t \subseteq B_{t+1}$.
- **Teorema do Zero Falso Negativo:**
  $$\forall k \in \text{InsertedKeys}: \quad I(k) \subseteq B_{\text{final}} \implies \text{may\_contain}(k) \equiv \text{true}$$
- O kernel implementa o oráculo de verificação que prova essa propriedade indutiva e falha imediatamente caso qualquer mutação de bits ou cálculo de índice viole o contrato de sondagem.

### P0.2: Invariante de Recuperação Dual WAL-MANIFEST (`dual_log_recovery_kernel.rs`)
- **Contrato de Corte de Replay:**
  O MANIFEST estabelece duas âncoras inegociáveis:
  1. $\text{min\_log\_number}$: Identificador do WAL mais antigo com dados não completamente persistidos em SSTables;
  2. $\text{earliest\_readable\_seq}$: O maior número de sequência cuja totalidade das mutações já foi consolidada em SSTables persistidas.
- **Teorema da Não-Ressurreição de Zumbis:**
  Dado um registro de WAL com $(\text{log\_num}, \text{seq}, k, v)$:
  $$\text{log\_num} < \text{min\_log\_number} \implies \text{Ação} = \text{DescartarLogObsoleto}$$
  $$\text{seq} \le \text{earliest\_readable\_seq} \implies \text{Ação} = \text{PularSequênciaConsolidada}$$
  $$\text{seq} > \text{earliest\_readable\_seq} \implies \text{Ação} = \text{AplicarNaMemtable}$$
  Garantia: Nenhuma mutação anterior à versão estabelecida pelo MANIFEST é reaplicada, impedindo categoricamente a ressurreição de dados deletados ou sobrescritos.

---

## 3. P1: Consistência Estrutural e Algébrica do LSM

### P1.1: Soundness de Purga de Tombstones (`tombstone_soundness_kernel.rs`)
- **Predicado de Purga Segura:**
  Um tombstone $(k, \text{seq}_{\text{del}})$ em compactação no nível $L$ só pode ser fisicamente descartado se e somente se:
  1. **Ausência de Snapshots Interceptores:**
     $$\forall s \in \text{ActiveSnapshots}: \quad \text{seq}_{\text{del}} < s$$
  2. **Ausência de Sombras em Níveis Inferiores:**
     $$\forall L' \in [L + 1, L_{\max}]: \quad \neg \text{LevelMayContainKey}(L', k)$$
- Se qualquer uma das condições falhar, o tombstone **deve** ser emitido no nível $L+1$. O kernel valida as compactações e intercepta qualquer tentativa de descarte prematuro que causaria desenmascaramento ou corrupção de snapshots.

### P1.2: Axioma de Transitividade Estrita do Comparador (`comparator_axiom_kernel.rs`)
- **Álgebra de Strict Weak Ordering:**
  Para qualquer comparador de chaves $C$ e quaisquer chaves arbitrárias $a, b, c \in \Sigma^*$:
  1. **Irreflexividade:** $\neg(a < a)$;
  2. **Assimetria:** $a < b \implies \neg(b < a)$;
  3. **Transitividade de $<$:** $(a < b \land b < c) \implies a < c$;
  4. **Transitividade de Equivalência:** Sendo $a \sim b \iff \neg(a < b) \land \neg(b < a)$, então $(a \sim b \land b \sim c) \implies a \sim c$.
- O kernel executa testes axiomáticos exaustivos sobre matrizes de chaves incluindo bytes nulos intercalados (`\0`), prefixos parciais, extremos de representação (`0x00`, `0xFF`) e payloads binários arbitrários.

---

## 4. P2: Contrato de Hardware e I/O Direto

### P2.1: Alinhamento Estrito de Direct I/O (`direct_io_contract_kernel.rs`)
- **Contrato de Alinhamento 4096-bytes:**
  Para qualquer operação de E/S direta (DMA sem cache do SO):
  $$\text{addr}(\text{buffer}) \pmod{4096} == 0$$
  $$\text{offset}(\text{arquivo}) \pmod{4096} == 0$$
  $$\text{length}(\text{transferência}) \pmod{4096} == 0$$
- O kernel fornece um container seguro `AlignedDirectBuffer` que aloca memória com garantia estrita de alinhamento a 4096 bytes sem ponteiros brutos arriscados, e um validador de requisições que rejeita em modo fail-closed qualquer chamada que viole a geometria de setores do hardware.

---

## 5. Esteira de Verificação Contínua (CVC Stage 15)

- Testes exaustivos unitários e de integração em `crates/pedradb-core/tests/rfc0281_bloom_dual_log_tombstone_comparator_dio.rs`.
- Verificação formal sob Miri (`-Zmiri-tree-borrows` e detecção de data-race).
- Adição formal do **Stage 15** em `scripts/verify_continuous_chain.sh`.
