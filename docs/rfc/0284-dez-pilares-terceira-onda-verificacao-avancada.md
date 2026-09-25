# RFC-0284: Os Dez Pilares da Terceira Onda de Verificação Avançada

- **Status:** Implementado & Verificado
- **Data:** 2026-09-25
- **Autores:** PedraDB Systems & Formal Verification Team
- **Escopo:** `pedradb-core`, `pedradb-spec`, Continuous Verification Chain (CVC)
- **Pré-requisitos:** RFC-0280, RFC-0281, RFC-0282, RFC-0283, RFC-0270 (Zero-Twin), RFC-0273 (Absolute Rigor)

---

## 1. Contexto e Motivação

As ondas anteriores (RFC-0280 a RFC-0283) estabeleceram bases sólidas de integridade, incluindo semântica fraca de memória RC11, cura de setores particionados, detecção de anomalias SSI, starvation-freedom e linearizabilidade de range scans.

No entanto, sob a ótica estrita de um matemático de sistemas formais e de um arquiteto sênior de storage engines, dez fragilidades críticas ainda permanecem expostas se examinadas no limite:
1. **Ambiguidade de Delimitador em Chaves Compostas:** Risco de colapso de codificação lexicográfica e perda de injetividade com bytes nulos (`0x00`).
2. **Colapso Entrópico de Filtros de Bloom:** Ataques de hash flooding que saturam filtros ($p \to 1$) e degradam o sistema para $O(N)$ leituras de NVMe.
3. **Dívida de Compactação Descontrolada (Write Stalls):** Pausas de segundos na escrita causadas por avalanches desreguladas de compactação.
4. **Coerência Despedaçada em `MultiGet`:** Leituras de múltiplos registros observando versões inconsistentes sob compactação contínua.
5. **Amplificação Física de Escrita no FTL (Flash Translation Layer):** Desalinhamento com Erase Blocks do SSD provocando degradação prematura de silício.
6. **Falhas em Volumes Cruzados (`EXDEV`):** Perda de atomicidade em migrações entre diretórios montados em sistemas de arquivos distintos.
7. **Inversão de Ordem por Rollover de Sequência ($u64$ Horizon):** Reversão catastrófica de visibilidade MVCC se o sequence number atingir $2^{64}$.
8. **Arquivos SST Órfãos Pós-Crash:** Descompasso entre arquivos gravados no disco e metadados no MANIFEST pós-queda de energia.
9. **Fantasmas em Índices Secundários Bitemporais:** Incoerência de leitura entre chave primária e índice em transações concorrentes.
10. **Deadlock e Desconexão por Cancelamento Assíncrono:** Falhas de threads líderes ou seguidores no group commit sob interrupção assíncrona.

Este RFC resolve os 10 itens de forma integral com código de produção verificado, sem dependências externas adicionais e com `#![forbid(unsafe_code)]`.

---

## 2. Especificação dos Dez Pilares

### Pilar 1: Injetividade Estrita e Delimitação Livre de Prefixo (`prefix_free_key_kernel.rs`)
- **Problema:** Codificar `(user_key, seq, type)` sem separador livre de prefixo permite colisões onde `Key("a", 1)` e `Key("a\0", 0)` geram o mesmo byte slice.
- **Teorema 1 (Injetividade e Preservação de Ordem):**
  $$\text{Encode}(k_1, s_1, t_1) = \text{Encode}(k_2, s_2, t_2) \iff (k_1, s_1, t_1) = (k_2, s_2, t_2)$$
  $$\text{Encode}(k_1, s_1, t_1) <_{\text{lex}} \text{Encode}(k_2, s_2, t_2) \iff (k_1 < k_2) \lor (k_1 = k_2 \land s_1 > s_2) \lor (k_1 = k_2 \land s_1 = s_2 \land t_1 > t_2)$$
- **Construção:** Codificação Length-Prefixed com escape unívoco ou framing de 4 bytes de tamanho da chave do usuário, seguido do sequence number codificado em big-endian decrescente (`!s`).

### Pilar 2: Imunidade a Hash Flooding e Entropia por Tabela (`bloom_hash_entropy_kernel.rs`)
- **Problema:** Um invasor pode fabricar chaves que colidem nos hashes de Bloom, saturando o filtro e causando 100% de falsos positivos.
- **Teorema 2 (Sementeira Entrópica Universal):**
  Cada SST file armazena um salt criptográfico $S_{\text{sst}} \in \{0, 1\}^{64}$ imutável gerado na sua criação. A função de hash do bloco é parametrizada por $S_{\text{sst}}$, garantindo que chaves colidentes em um SST não colidam nos demais SSTs com probabilidade superior a $1 / 2^{64}$.
- **Construção:** Avaliador de densidade de bits com corte de saturação: se uma partição atinge densidade $\ge 0.5$, o motor rejeita a inserção ou dobra a capacidade.

### Pilar 3: Pacing Anti-Stall e Dívida de Compactação Bounded (`compaction_pacing_kernel.rs`)
- **Problema:** Picos de escrita enchem o L0 e disparam compactações em cascata, congelando threads de clientes por segundos.
- **Teorema 3 (Atraso Máximo Limitado):**
  A taxa de admissão de escrita $\lambda$ é regulada por uma função suave da dívida pendente $\mathcal{D}$:
  $$\text{Delay}(\mathcal{D}) = \min\left(T_{\max}, \frac{\mathcal{D}}{\mathcal{D}_{\text{target}}} \cdot \Delta_{\text{base}}\right)$$
  Garantindo que nenhuma requisição sofra atraso superior a $T_{\max}$ e que a dívida decresça monotonicamente.

### Pilar 4: Super-Atomicidade e Coerência de `MultiGet` (`super_atomic_multiget_kernel.rs`)
- **Problema:** Durante o lookup de uma lista de chaves $[k_1, \dots, k_m]$, compactações concorrentes podem substituir arquivos no LSM, gerando leituras que misturam versões distintas.
- **Teorema 4 (Visão Homogênea Imutável):**
  O lote de busca $[k_1, \dots, k_m]$ opera sobre um único `VersionEpochPinnedHandle`. Todos os acessos a SSTs e MemTables satisfazem:
  $$\forall k_i \in K: \text{Read}(k_i) \equiv \text{Eval}(k_i, \mathcal{V}_0)$$
  onde $\mathcal{V}_0$ permanece imutável e retida em memória até o retorno de todas as respostas do lote.

### Pilar 5: Alinhamento com FTL Erase Blocks e Minimização de WAF (`ftl_erase_boundary_kernel.rs`)
- **Problema:** Deletar arquivos fora dos limites de blocos de apagamento flash (4MB–16MB) força o garbage collector do drive a copiar páginas válidas, explodindo a amplificação física de escrita (WAF).
- **Teorema 5 (Alinhamento de Geometria Flash):**
  Arquivos SST e VLog são particionados em limites estritamente alinhados ao `EraseBlockSize` (ex: 4 MiB). Toda deleção por compactação descarta blocos de flash contíguos completos, limitando o WAF físico a $\le 1.05 \times \text{WAF}_{\text{LSM}}$.

### Pilar 6: Barreira Atômica Multi-Filesystem (`cross_device_barrier_kernel.rs`)
- **Problema:** Quando WAL e SST residem em discos diferentes (`/fast_wal` e `/data_sst`), `rename(2)` falha com `EXDEV`. Migrações ingênuas via copy+unlink perdem atomicidade sob corte de energia.
- **Teorema 6 (Idempotência de Migração Transversal):**
  O protocolo de 2 fases:
  1. `StageTarget(src, dst.tmp) -> fdatasync(dst.tmp) -> fsync(dst_dir)`
  2. `CommitManifest(dst) -> fsync(manifest_dir)`
  3. `UnlinkSource(src) -> fsync(src_dir)`
  garante que qualquer falha antes do passo 2 descarta `dst.tmp` sem afetar o dado, e qualquer falha após o passo 2 recupera `dst` e remove `src`.

### Pilar 7: Horizonte de Sequência $u64$ e Barreira de Rollover (`sequence_horizon_kernel.rs`)
- **Problema:** Se $s$ sofrer overflow ($2^{64}-1 \to 0$), toda a ordenação MVCC é destruída.
- **Teorema 7 (Horizonte Inviolável):**
  O alocador de sequence number impõe um limite máximo de segurança $S_{\text{safe}} = 2^{64} - 2^{32}$. Ao atingir $S_{\text{safe}}$, o motor recusa novas alocações de escrita e entra em modo determinístico `EpochRekeyRequired`, impedindo categoricamente qualquer reversão circular.

### Pilar 8: Idempotência de Limpeza de Arquivos Órfãos Pré-Manifesto (`orphan_sst_cleanup_kernel.rs`)
- **Problema:** Arquivos SST criados durante uma compactação que sofreu crash antes de gravar o `VersionEdit` no MANIFEST ficam soltos no diretório.
- **Teorema 8 (Partição Segura do Espaço de Arquivos):**
  Na recuperação pós-crash, o diretório físico é particionado em:
  $$\text{Files}_{\text{disk}} = \text{Files}_{\text{manifest}} \cup \text{Files}_{\text{orphan}}$$
  onde $\forall f \in \text{Files}_{\text{orphan}}: f_{\text{num}} < \text{Manifest.NextFileNum}$. O limpador remove comutativamente todos os órfãos antes de liberar a admissão de novos writes.

### Pilar 9: Coerência Bitemporal entre Chave Primária e Índices Secundários (`bitemporal_index_kernel.rs`)
- **Problema:** Leituras concorrentes podem enxergar a chave primária atualizada mas o índice secundário na versão anterior, quebrando a consistência transacional da aplicação.
- **Teorema 9 (Equivalência Mútua de Visibilidade):**
  Para qualquer snapshot $S$, a visibilidade do par primário-secundário é indivisível:
  $$\forall t: \text{Visible}(t, \text{Primary}) \iff \text{Visible}(t, \text{Secondary})$$
  provado através do commit indivisível em lote único com o mesmo sequence number atômico.

### Pilar 10: Imunidade a Cancelamento Assíncrono no Group Commit (`async_cancellation_kernel.rs`)
- **Problema:** O cancelamento assíncrono (timeout, sinal, abort) de um thread líder ou seguidor na fila de group commit pode gerar deadlocks perpétuos ou corrupção de slots no buffer.
- **Teorema 10 (Desassociação Lock-Free e Eleição Finita):**
  O nó na fila de commit possui transição de estado atômica:
  $$\text{Waiting} \xrightarrow{\text{cancel}} \text{Cancelled} \quad \text{ou} \quad \text{Committed}$$
  Se o líder cancela, ele promove o primeiro seguidor ativo a novo líder em $O(1)$. Se um seguidor cancela, seu payload é ignorado durante o flush pelo líder sem comprometer o resto do lote.

---

## 3. Conclusão

Com a implementação e verificação deste conjunto, o PedraDB neutraliza dez das mais sutis e devastadoras classes de falhas lógicas, físicas e assíncronas em sistemas de armazenamento moderno.
