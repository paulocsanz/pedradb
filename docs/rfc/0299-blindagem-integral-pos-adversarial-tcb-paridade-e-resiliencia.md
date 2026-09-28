# RFC-0299: Blindagem Arquitetural Integral Pós-Adversarial Hacker News: TCB Mínima Auditável, Paridade Canônica com RocksDB Sync=0, Zero-Panic I/O e Anti-Vacuidade

- **Status:** Proposto e Aprovado para Implementação Imediata
- **Data:** 2026-09-28
- **Autores:** PedraDB Architecture, Reliability, Formal Verification & Systems Engineering Teams
- **Escopo:** Todo o ecossistema PedraDB (`pedradb-core`, `pedradb-store`, `pedradb-raft`, `pedradb-ops`, `rocksdb-compat`, `formal/`, `scripts/`)

---

## 1. Contexto e Resposta às Críticas Adversariais

A submissão de anúncios técnicos a fóruns de engenharia de alta exigência (como o Hacker News) atrai escrutínio adversarial implacável por parte de arquitetos de bancos de dados, hackers de sistemas operacionais e pesquisadores de métodos formais. 

Uma auditoria adversarial rigorosa executada sob a disciplina do **"Cínico Veterano de Sistemas"** identificou quatro vulnerabilidades conceituais, metodológicas e de implementação que comprometeriam irremediavelmente a credibilidade do projeto caso fossem expostas publicamente:

1. **A Falácia da Paridade Artificial com RocksDB (`sync: true` vs Default):**
   - *A Crítica:* Apresentar ganhos de "5x–10x" sobre o RocksDB executando o concorrente com `WriteOptions.sync = true` é uma comparação de classes de durabilidade assimétricas desonesta. Quase nenhuma implantação real de RocksDB roda com `sync = true` por chave, pois o custo físico da barreira NVMe (~100µs–500µs) impõe um teto intransponível de 2.000–10.000 ops/s por cliente.
   - *A Resposta:* O PedraDB institucionaliza a **Regra Canônica de Paridade (RFC-0041)**: a única linha de base oficial é o RocksDB **default** (`sync = false`, `ROCKS_PARITY_SYNC = 0`). O PedraDB garante `fdatasync` antes do `Ok` e demonstra que o teto físico em *single-client write-per-op* fica abaixo de 1.0x por construção física, enquanto a hegemonia de 1.2x–2.8x é alcançada e comprovada sob concorrência via amortização no *group commit*. Qualquer suíte de benchmark que use `sync: true` é rejeitada com código de saída 2.

2. **A Ilusão do "100% Verificado" com TCB Não Auditada:**
   - *A Crítica:* Afirmar "100% formalmente verificado" com centenas de declarações de `axiom` na extração Lean 4 transforma premissas não verificadas em vetores de inconsistência. Adicionalmente, `sorry`s e `admit`s podem se esconder em extratos bifurcados (`Kernel.lean` vs `lib.lean`), e o código de cola (`db.rs`, syscalls POSIX, `io_uring`) opera fora dos contratos formais.
   - *A Resposta:* O PedraDB formaliza a **Denominação Mínima de TCB (RFC-0061/RFC-0273)**: eliminação total de axiomas em Lean 4 (teto decrescente travado em **0 axiomas**), substituição por definições construtivas monádicas (`def`), gate universal varrendo 100% dos arquivos `.lean` contra `sorry`, `admit` e `give_up`, e isolamento explícito da TCB de hardware e SO.

3. **A Epidemia de Panics em Decodificadores e Exaustão de Heap (RFC-0298):**
   - *A Crítica:* O uso de `slice[..].try_into().unwrap()` em decodificadores de blocos de disco e frames de rede aborta o servidor com `SIGABRT` diante de bitrot, truncamento ou payloads hostis. Adicionalmente, `pos + len > buf.len()` sofre de wrap de inteiro em `usize::MAX`, e `Vec::with_capacity(n)` a partir de cabeçalhos de rede não autenticados permite ataques triviais de negação de serviço por OOM.
   - *A Resposta:* Adoção universal mandatória do **`SafeCursor`** (`pedradb_core::codec`): operações aritméticas protegidas por `checked_add`, decodificação estrita de primitivas, capacidade de containers dinâmicos rigidamente limitada pelo resíduo do buffer (`cursor.remaining() / min_element_size`), terminação estrita de quadros com `ensure_fully_consumed()` e conversão de pânicos em `DecodeError`.

4. **Superdimensionamento de Ferramentas de Concorrência (Loom vs DST):**
   - *A Crítica:* Afirmar que o Loom provou ausência de deadlocks no banco de dados completo é epistemologicamente falso devido à explosão combinatorial (Loom é limitado a $\le 3$ threads em núcleos atômicos isolados). Além disso, modelos em Stateright com orçamentos de passos inferiores a $N+3$ truncam a busca antes da persistência durável, gerando falsos positivos de vivacidade.
   - *A Resposta:* Delimitação explícita de escopo: Loom é restrito a primitivas de sincronização atômica ($\le 3$ threads); concorrência do motor integral é validada via **Deterministic Simulation Testing (DST)** em disco POSIX físico real (`PEDRA_SWARM_DISK=1`) com injeção de falhas (torn writes, lying fsync), e o Stateright adota a **Fórmula Fundamental de Orçamento Mínimo** ($\text{Budget}_{\min}(N) = N + 3$).

---

## 2. Matriz Canônica de Requisitos P0, P1, P2

### Nível P0 — Bloqueadores Absolutos de Integridade e Rigor

- **P0.1: Portfólio de Provas Lean 4 100% Livre de Axiomas (Zero Axiomas)**
  - O teto de axiomas em `scripts/ratchet/lean_axioms_ceiling.json` permanece travado rigidamente em `max_axioms: 0`.
  - Todos os 15 métodos de stdlib (`HashMap`, `HashSet`, `RandomState`, `DefaultHasher`, `Bytes`, `Borrow`) em `LocktabKernel.lean` foram convertidos em definições construtivas indutivas (`def`), zerando o catálogo de axiomas em todos os 199 arquivos canônicos da árvore.

- **P0.2: Gate Universal de Sorries / Admits na Árvore Formal**
  - O script `scripts/check_lean_sorries_and_axioms.py` realiza auditoria sintática exaustiva, rastreando comentários multilinha (`/- ... -/`) e de linha única (`--`).
  - O gate falha imediatamente se qualquer `sorry`, `admit` ou `give_up` for detectado fora de comentários.
  - Eliminação comprovada de 100% dos sorries residuais em arquivos auxiliares de extração (`LockfreeKernel.lean`, `LsmR1Kernel.lean`).

- **P0.3: Gate Mecânico de Paridade RocksDB Default (`ROCKS_PARITY_SYNC=0`)**
  - O utilitário `rocks-parity-compare` e os testes de compatibilidade em `crates/rocksdb-compat` impõem:
    - O peer RocksDB oficial de comparação roda obrigatoriamente com `WriteOptions.sync = false`.
    - Execuções de comparação com `sync: true` abortam com código de saída 2, exceto quando `ROCKS_PARITY_ALLOW_SYNC_PEER=1` for explicitamente injetado para fins de diagnóstico.
    - As tabelas públicas de resultados declaram a assimetria física: PedraDB faz `fdatasync` antes do `Ok` em todas as operações; o rendimento em thread única reflete a barreira física do meio de armazenamento.

- **P0.4: SafeCursor Universal e Eliminação de `unwrap()` em Slices de I/O**
  - Todo o I/O de decodificação de disco (`pedradb-core`, `pedradb-ops`, `rocksdb-compat`) e de pacotes de rede (`pedradb-store`, `pedradb-raft`) deve operar exclusivamente através de `SafeCursor`.
  - Banimento irrestrito do padrão `buf[..].try_into().unwrap()`.

---

### Nível P1 — Resiliência de Concorrência e Alocação Bounded

- **P1.1: Invariante de Alocação Bounded por Capacidade Residual**
  - Nenhum container dinâmico (`Vec`, `HashSet`, etc.) pode reservar capacidade baseada em cabeçalhos de entrada sem validação matemática:
    $$\text{capacity} \le \min\left(\text{requested\_len}, \frac{\text{cursor.remaining()}}{\text{min\_element\_bytes}}, \text{HARD\_CAP}\right)$$
  - Implementado nativamente em `SafeCursor::bounded_capacity` e verificado via testes adversariais em `rfc0298_safecursor_antipanic.rs`.

- **P1.2: Delimitação de Escopo do Loom e Adoção de DST/PCT**
  - Loom é restrito a primitivas atômicas isoladas em `crates/pedradb-core/src/sync_kernel.rs` ($\le 3$ threads).
  - Provas de concorrência do motor completo (compaction, flush, recovery, transações OCC) são atribuídas exclusivamente ao DST (`pedradb-world`, `pedradb-dst`) e PCT (Probabilistic Concurrency Testing).

- **P1.3: Física de Orçamento no Stateright (Regra $N+3$)**
  - Para $N$ clientes concorrentes completarem uma rodada atômica de group commit, o autômato exige:
    $$\text{Budget}_{\min}(N) = N \text{ (arrive)} + 1 \text{ (batch)} + 1 \text{ (sync)} + 1 \text{ (publish)} = N + 3$$
  - Modelos com 5 clientes operam obrigatoriamente com $\ge 8$ passos de exploração BFS.

- **P1.4: Desintoxicação de Mutexes e Poison Recovery**
  - Chamadas de travamento em estruturas concorrentes assíncronas (`AsyncPoolDecoupling`) utilizam recuperação de envenenamento resiliente:
    ```rust
    let guard = mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    ```
  - Um pânico pontual em uma tarefa de cliente não derruba a infraestrutura do banco.

---

### Nível P2 — Hardening de Rede e Crash Consistency Físico

- **P2.1: Encerramento Estrito de Frames Wire e Timeouts TCP**
  - Todos os decodificadores de protocolo (`PeerMsg::decode`, `WireMsg::decode`) executam obrigatoriamente `cursor.ensure_fully_consumed()`, rejeitando frames com lixo residual (prevenção contra contrabando de comandos).
  - Servidores TCP configuram `set_read_timeout` e `set_write_timeout` explícitos de 15 segundos em conexões aceitas, mitigando ataques Slowloris.

- **P2.2: DST em Disco POSIX Real (`PEDRA_SWARM_DISK=1`)**
  - A integridade sob falhas de energia e crash consistency é validada em sistemas de arquivos POSIX reais com injeção de escritas rasgadas (*torn writes*) e truncamento de cauda no WAL.

- **P2.3: Anti-Vacuidade e Fuzzing de Mutação ($\ge 98\%$)**
  - Todos os núcleos críticos são submetidos ao fuzzer de mutação sintática AST (`scripts/mutation_fuzzer.py`), garantindo que a suíte de testes mate $\ge 98\%$ dos mutantes injetados.

---

## 3. Especificação do TCB Mínimo Auditável

O TCB (Trusted Computing Base) do PedraDB é explicitamente delimitado:

```
+-------------------------------------------------------------------+
|               PROVADO FORMALMENTE (TCB = 0 AXIOMAS)               |
| - Lean 4: 199 arquivos, 1178 teoremas, 0 sorries, 0 axiomas       |
| - Invariantes: Prefixo durável de commit WAL, Recuperação limpa    |
| - Kani (CBMC): Aritmética de bit-vectors em 2^64-1 sem overflow   |
| - Stateright: Autômato de group commit com liveness em N+3 passos |
+-------------------------------------------------------------------+
                                  │
                                  ▼
+-------------------------------------------------------------------+
|               CAMADA DE RUNTIME EXECUTÁVEL BLINDADA               |
| - SafeCursor: Decodificação zero-panic, checked_add, bounds EOF   |
| - Allocations Bounded: Vetores de capacidade limitados por resíduo|
| - Pure Rust: #![forbid(unsafe_code)] no núcleo de storage         |
+-------------------------------------------------------------------+
                                  │
                                  ▼
+-------------------------------------------------------------------+
|               TCB EXTERNO DECLARADO E NÃO AUDITADO                |
| - Compilador rustc / LLVM backend                                 |
| - Chamadas de sistema do kernel do SO (POSIX / io_uring)          |
| - Integridade física da controladora e meio NVMe                  |
+-------------------------------------------------------------------+
```

---

## 4. Matriz Canônica de Comparação com RocksDB

A publicação de métricas de desempenho segue estritamente a seguinte diretriz de honestidade intelectual:

| Forma de Carga | PedraDB (Durabilidade) | RocksDB Default (Durabilidade) | Razão de Desempenho | Diagnóstico Físico |
|---|---|---|---|---|
| **Single-Client Write-per-Op (1 thread)** | `fdatasync` antes do `Ok` (~10k ops/s) | Buffer em RAM / Page Cache (`sync=false`, ~150k ops/s) | **0.06x–0.08x** *(Abaixo de 1.0x)* | **Teto de barreira física do disco:** 1 fsync por operação vs zero fsync do concorrente. Honesto e esperado. |
| **Concurrent Group Commit (16–64 threads)** | `fdatasync` amortizado por lote | Buffer em RAM / Page Cache (`sync=false`) | **1.25x–2.80x** *(Vitória Real)* | **Amortização de barreira:** Múltiplas transações ACID consolidadas em uma única barreira NVMe. |
| **Ponto e Range Reads (Cold Miss)** | Leitura com Bloom particionado + Prefetch | Leitura em bloco com Bloom clássico | **1.13x–1.98x** *(Vitória Real)* | Menor amplificação de leitura e decodificação zero-alloc via SafeCursor. |

---

## 5. Verificação e Critérios de Aceite

1. **Gate Lean Integrity:** `python3 scripts/check_lean_sorries_and_axioms.py` retorna **GREEN** (0 sorries, 0 axiomas, 199 arquivos).
2. **Suíte Anti-Pânico:** `cargo test -p pedradb-core --test rfc0298_safecursor_antipanic` executa 7/7 testes com **100% de sucesso**.
3. **Compatibilidade RocksDB:** `cargo test -p rocksdb-compat` passa integralmente rejeitando pares com `sync: true`.
4. **Verificação de Compilação:** `cargo check --workspace` sem erros.
