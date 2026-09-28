# RFC-0295: Arquitetura de Fechamento Integral do TCB: Unificação de Refinamento Top-Level, Concorrência Linearizável, Blindagem de Glue e Deterministic Simulation Testing (DST)

- **Status:** Proposto e Aprovado para Implementação
- **Data:** 2026-09-28
- **Autores:** PedraDB Architecture, Concurrency & Formal Verification Teams
- **Escopo:** Todo o workspace PedraDB (`pedradb-core`, `pedradb-store`, `rocksdb-compat`, `pedradb-capi`, `pedradb-replicate`, `pedradb-http`, `formal/`, `scripts/`)

---

## 1. Contexto e Motivação

O PedraDB acumulou um corpo substancial de verificação formal (286 átomos matemáticos provados em Lean 4 via extração Aeneas/Charon e 18 kernels verificados via Verus). Não obstante, auditorias e caças recentes de vulnerabilidades revelaram 20 anomalias críticas em produção que escaparam incólumes aos provadores de teoremas:

1. **A Ilusão da Cobertura Formal:** Apenas 17% do código-fonte (27.912 LOC de kernels puros) possui modelos matemáticos. Os restantes 83% (~136.500 LOC) consistem em código de colagem (*glue code*), concorrência multithreaded, I/O físico de rede e disco, e interfaces C/FFI, onde residem os bugs reais.
2. **Caminhos Rápidos Desacoplados (*Bypass Antipattern*):** Otimizações em produção (`fast_outside_sst_miss`, `commit_async_one_syscall_off_lock`) contornavam os kernels formais para atingir métricas de throughput, introduzindo corridas de linearizabilidade e leituras fantasmas.
3. **Abismo Semântico de I/O:** Axiomas formais presumindo integridade física e completude de buffers colidiram com o comportamento real do POSIX (truncamento de WAL a 0 bytes reciclado, leituras parciais em `read_exact`, conexões HTTP presas em Slowloris, e quebras de alinhamento de 32 KB sem padding).
4. **Violações de Invariantes em Bordas de Integração:** Erros estruturais na camada de compatibilidade RocksDB e na C-API do FoundationDB (destruição prematura de handles em retries, confusão de Column Families em locks, ranges invertidos `start > end`, e vazamentos de heap).
5. **Fragilidade do Pipeline de CI:** Tolerância a saídas `skip -> exit 0` na ausência de toolchains locais e ausência de compilação/checagem contínua de Lean no GitHub Actions.

Este RFC estabelece a refundação estrutural necessária para fechar permanentemente o Trusted Computing Base (TCB), eliminar a fissura entre prova matemática e execução física, e garantir que nenhuma linha de código em produção viole a semântica formal do banco.

---

## 2. Princípios Fundamentais do Novo TCB

### Princípio I: Dreno Total de Glue (*Pure State-Machine & Zero-Logic Glue*)
O código fora dos kernels verificados é reduzido à condição de **trampolim estúpido** (*dumb driver*). Toda e qualquer lógica de decisão:
- Decisão de quebra de arquivos SST (`compact_should_split_at`),
- Destino e ciclo de vida de tombstones (`lone_tombstone_fate`, `GcMergeSource`),
- Avanço de cursores e rotação de logs em replicação (`stamp_changed`),
- Roteamento e qualificação de Column Families,
deve residir exclusivamente dentro de funções puras modeladas formalmente, recebendo estado imutável e retornando decisões tipadas. A camada não-verificada limita-se a despachar bytes para syscalls de I/O.

### Princípio II: Tipagem de Evidência e Invariantes na Compilação (*Parse, Don't Validate*)
Estados ilegais são tornados inexprimíveis pelo compilador Rust. Parâmetros soltos em APIs públicas e internas são substituídos por tipos com invariantes seladas:
```rust
/// Intervalo estritamente ordenado garantido por construção
pub struct ValidatedRange<'a> {
    start: &'a [u8],
    end: &'a [u8],
}

impl<'a> ValidatedRange<'a> {
    #[inline]
    pub fn try_from_slice(start: &'a [u8], end: &'a [u8]) -> Result<Self, Error> {
        if start > end {
            return Err(Error::InvalidArgument("start key exceeds end key"));
        }
        Ok(Self { start, end })
    }
}
```

### Princípio III: Token de Linearizabilidade e Sequência Visível
A publicação do número de sequência monótono para leitores só pode ser realizada através da entrega de um comprovante de inserção física na MemTable (`MemtableCommitProof`):
```rust
pub struct MemtableCommitProof(u64);

impl MemTable {
    pub fn insert_batch(&self, seq: u64, batch: &WriteBatch) -> MemtableCommitProof {
        self.raw_insert(seq, batch);
        MemtableCommitProof(seq)
    }
}

impl VersionSet {
    // É impossível publicar visible_seq sem apresentar a prova de que a Memtable já absorveu a escrita
    pub fn publish_sequence(&self, proof: MemtableCommitProof) {
        self.visible_sequence.fetch_max(proof.0, Ordering::Release);
    }
}
```

---

## 3. Especificação das Resoluções dos 20 Problemas Identificados

### 3.1 Concorrência e MVCC (`pedradb-core`)

1. **Eliminação do Bypass em `fast_outside_sst_miss`:**
   - *Regra:* É estritamente proibido retornar `None` com base nos envelopes das SSTables sem antes esgotar a busca na MemTable ativa e nas MemTables imutáveis.
   - *Correção:* A checagem de envelopes SST é rebaixada para o sub-módulo de busca em disco `TableCache::get()`, executada somente após o miss confirmado em memória.

2. **Publicação Linearizável de `visible_seq`:**
   - *Regra:* Em `commit_async_one_syscall_off_lock`, o incremento de `seq` reserva o espaço lógico, mas o leitor só enxerga `visible_seq`.
   - *Correção:* O leitor de snapshot obtém `min(snapshot_seq, db.visible_seq.load(Acquire))`. A atualização de `visible_seq` é realizada após a inserção bem-sucedida em todas as memtables ativas.

### 3.2 Persistência e Estrutura SST (`pedradb-core`)

3. **Garantia de Framing de Bloco de 32 KB no WAL:**
   - *Regra:* Nenhum registro de WAL pode cruzar blocos de 32 KB sem o preenchimento explícito dos bytes residuais inferiores a 7 bytes com `RecordType::Zero`.
   - *Correção:* Em `WalWriter::encode_one_op_full_detached`, quando `block_rem < HEADER_SIZE (7)`, grava-se um buffer preenchido com zeros até o final do bloco e reinicia-se o acumulador no offset alinhado.

4. **Fronteira Absoluta de Tamanho em Compatação:**
   - *Regra:* Arquivos SST não podem crescer indefinidamente sob chaves idênticas repetidas.
   - *Correção:* Em `compact_should_split_at`, introduz-se um limite máximo estrito `hard_file_size_limit = target_file_size * 2`. Ao atingir esse limite, a quebra é compulsória mesmo que `curr_key == prev_key`.

5. **Invariante de Retenção de Versão no `GcMergeSource`:**
   - *Regra:* Nenhuma versão com `version_seq >= oldest_active_snapshot` pode ser descartada.
   - *Correção:* Modifica-se o predicado de elegibilidade de GC:
     $$\text{DropCandidate}(v) \iff v.\text{seq} < \min(\text{ActiveSnapshots}) \land \exists v_{\text{newer}} \text{ preservada com } v_{\text{newer}}.\text{seq} \le v.\text{seq}$$

6. **Conservação Global de Tombstones em Níveis Superiores:**
   - *Regra:* O descarte de tombstones (`lone_tombstone_fate`) exige prova exaustiva de ausência da chave em todos os níveis mais profundos ($L+1 \dots L_{\max}$).
   - *Correção:* Consultar o resumo de filtros de Bloom e ranges de chaves de todos os níveis inferiores antes de autorizar a remoção física do tombstone.

7. **Proteção de Ingestão e SST Vazia:**
   - *Regra:* Arquivos SST sem chaves de usuário ativas não geram metadados inválidos de fronteira.
   - *Correção:* Em `write_bulk_sst` e `disjoint_sorted_by_lo`, tratar `keys.is_empty()` retornando `None` de forma segura, sem invocar indexação cega `keys[0]` nem `.unwrap()`.

8. **Cálculo Canônico do Trailer de Bloco Comprimido:**
   - *Regra:* O offset físico e o `BlockHandle::length` devem refletir com precisão matemática o tamanho comprimido + flag de compressão (1 byte) + CRC32C (4 bytes).
   - *Correção:* Ajustar a codificação de bloco para unificar a escrita física e a emissão do handle no índice via estrutura RAII `BlockWriter`.

9. **Eliminação de `panic!` em Leitura Corrompida:**
   - *Regra:* Falhas físicas de leitura ou corrupção de checksum devem propagar `Result::Err(Error::Corruption)` para permitir isolamento e recuperação da réplica.
   - *Correção:* Substituir chamadas a `fail_stop_corrupt_value` e `fail_stop_corrupt_block` por retornos formais de erro estruturado.

### 3.3 Camada RocksDB e Compatibilidade Transacional (`rocksdb-compat`)

10. **Preservação de Estado Transacional em Falhas de Commit:**
    - *Regra:* `Transaction::commit()` não pode destruir o buffer de mutações em caso de erro recuperável (conflito de concorrência ou I/O transitório).
    - *Correção:* O esvaziamento do lote de escritas ocorre exclusivamente após a confirmação de sucesso do commit pelo engine subjacente.

11. **Desalocação Imediata de Snapshot em `rollback()`:**
    - *Regra:* O cancelamento de uma transação deve desregistrar o snapshot retido imediatamente.
    - *Correção:* Em `Transaction::rollback()`, invocar explicitamente `self.pin.take()` e notificar a lista global de snapshots ativos para desbloquear o GC.

12. **Isolamento Estrito de Column Family em `get_for_update_cf`:**
    - *Regra:* A codificação de chaves e o sistema de travas devem incorporar obrigatoriamente o identificador numérico da Column Family (`cf_id`).
    - *Correção:* Substituir chamadas a `self.encode(key)` por `self.encode_cf(cf, key)`.

13. **Validação de Limites em `delete_range_cf`:**
    - *Regra:* Rejeitar ranges invertidos na fronteira da API.
    - *Correção:* Encapsular os parâmetros em `ValidatedRange` e rejeitar chamadas onde `start > end`.

### 3.4 C-API e FFI (`pedradb-capi`)

14. **Gestão RAII de Alocações na C-API:**
    - *Regra:* Buffers alocados no heap nativo via FFI devem ser rastreados para evitar vazamentos em caso de erro intermediário.
    - *Correção:* Implementar guardião `ScopedCBytes` que libera o ponteiro no `drop` caso não ocorra transferência explícita de posse via `into_raw()`.

15. **Ciclo de Vida Conforme a Especificação FDB C-API:**
    - *Regra:* O handle da transação (`fdb_transaction`) só pode ser desregistrado do mapa em `fdb_transaction_destroy`.
    - *Correção:* Em `montanha_fdb_transaction_commit`, manter o handle registrado mesmo em falha de commit, permitindo que a aplicação cliente execute o retry padrão via `fdb_transaction_on_error`.

### 3.5 Replicação, Transporte e I/O (`pedradb-replicate`, `pedradb-http`, `pedradb-store`)

16. **Detecção Injetiva de Rotação de Logs (`stamp_changed`):**
    - *Regra:* A identidade de um arquivo WAL é dada pela tupla $\langle \text{FileId}, \text{Inode}, \text{FileSize}, \text{HeaderHash} \rangle$.
    - *Correção:* Eliminar comparações baseadas apenas em prefixos truncados. Se o arquivo encolheu ou se o inode/mtime foi alterado, disparar re-sincronização imediata.

17. **Atomicidade no Avanço de Cursor de Replicação:**
    - *Regra:* O cursor de replicação (`self.offset`) só pode ser avançado após a leitura física completa dos bytes requeridos.
    - *Correção:* `f.read_exact(&mut buf)?` antes de qualquer mutação de estado. Em caso de erro, o cursor permanece intacto.

18. **Mitigação Definitiva de Slowloris no Transporte HTTP:**
    - *Regra:* Todo socket de rede aceito deve operar sob limites rígidos de timeout para leitura de cabeçalhos e corpo.
    - *Correção:* Configurar `set_read_timeout(Some(Duration::from_secs(5)))` e `set_write_timeout(Some(Duration::from_secs(10)))` imediatamente após o `accept()`.

19. **Transacionalidade na Persistência de Metadados SI (`persist_si_keys`):**
    - *Regra:* A atualização de números de geração de Snapshot Isolation entre réplicas deve ser atômica e confluente.
    - *Correção:* Submeter a mudança de geração através do log unificado do Raft (`pedradb-raft`), garantindo consenso de quorum antes da ativação local.

---

## 4. Arquitetura de Validação: Deterministic Simulation Testing (DST)

Para garantir que bugs sutis de concorrência e I/O caótico nunca mais alcancem a produção, institui-se o **SimSystem DST** integrado ao `pedradb-world`:

```
┌────────────────────────────────────────────────────────┐
│           Deterministic Simulation Testing             │
│                                                        │
│  ┌─────────────────┐ ┌─────────────────┐ ┌──────────┐  │
│  │ Simulated Time  │ │ Simulated Disk  │ │ Simulated│  │
│  │ (Virtual Clock) │ │ (Torn Writes/IO)│ │ Network  │  │
│  └────────┬────────┘ └────────┬────────┘ └────┬─────┘  │
│           └───────────────────┼───────────────┘        │
│                               ▼                        │
│                 Deterministic Scheduler                │
│             (PCT / Systematic Interleaving)            │
│                               ▼                        │
│             PedraDB Kernel Under Chaos Testing         │
└────────────────────────────────────────────────────────┘
```

1. **Disco Virtual Determinístico:** Simula setores defeituosos, escritas parciais (*torn writes* em falhas de energia), erros intermitentes de `read_exact` e atrasos de `fsync`.
2. **Relógio e Concorrência Determinísticos:** Todas as threads do banco passam a ser tarefas em um escalonador cooperativo monofilar dirigido por seed aleatória.
3. **Invariante de Oráculo:** A cada transação e compaction, o estado do banco é comparado contra um oráculo em memória (`BTreeMap` trivial). Qualquer desvio de linearizabilidade dispara pânico imediato com a semente exata para reprodução em 100 milissegundos.

---

## 5. Reforma do Pipeline de Verificação Formal e CI

1. **Erradicação do Modo Permissivo:**
   - Todos os scripts em `scripts/verus_*.sh`, `scripts/aeneas_*.sh` e `scripts/lean_*.sh` devem falhar com código de erro não-nulo (`set -euo pipefail`) caso dependências ou provadores estejam ausentes (`--required` obrigatório).
2. **Integração Completa no GitHub Actions:**
   - Adicionar o workflow `formal-verification-matrix.yml` no GitHub Actions executando Verus (pinado), Lean 4 / Mathlib e Charon/Aeneas a cada Pull Request.
3. **Caller-Lint Bloqueante no Pré-Commit:**
   - Implementar verificação estática que rejeita qualquer código em `pedradb-core` ou `pedradb-store` que tente duplicar internamente uma lógica para a qual já existe um kernel provado.

---

## 6. Plano de Ação e Fases de Implementação

| Fase | Ações Principais | Prazo Estimado |
|---|---|---|
| **Fase 1: Correção Imediata** | Consertar os 20 bugs identificados no código Rust, remover bypasses de memtable e isolar Column Families. | 3 dias |
| **Fase 2: Tipagem e Invariantes** | Introduzir `ValidatedRange`, `MemtableCommitProof` e guardiões RAII na C-API. | 5 dias |
| **Fase 3: Expansão do CI e Lint** | Adicionar Lean e Verus obrigatórios no CI sem `skip`; habilitar *caller-lint* bloqueante. | 4 dias |
| **Fase 4: DST Completo** | Expandir testes determinísticos no `pedradb-world` com injeção caótica de falhas de I/O e crash recovery. | 7 dias |

---

## 7. Conclusão

Com a adoção do RFC-0295, o PedraDB transcende a falsa segurança de "provas matemáticas em ilhas isoladas". Ao drenar a complexidade do *glue code*, impor invariantes irrefutáveis na tipagem de compilação, fechar os limites de I/O e submeter o motor a testes de simulação determinística contínua, o banco atinge a real equivalência de confiabilidade aos sistemas de missão crítica de classe mundial.
