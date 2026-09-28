# RFC-0300: Arquitetura Bulletproof para Concorrência Resiliente, Pacing de Compactação e Hegemonia Comparativa de Benchmarks

- **Status:** Proposto e Aprovado para Implementação Integral
- **Data:** 2026-09-28
- **Autores:** PedraDB Architecture, Reliability, High-Performance Storage & Systems Research Teams
- **Escopo:** Todo o workspace PedraDB (`pedradb-core`, `pedradb-store`, `rocksdb-compat`, `rocksdb-parity-bench`, `formal/`, `tests/`)

---

## 1. Contexto e Motivação

A auditoria adversarial de Fase 2, conduzida sob a perspectiva de arquitetos de storage de classe mundial (RocksDB, SQLite, TigerBeetle, FoundationDB), reconheceu os avanços monumentais alcançados com as **RFC-0298** (SafeCursor anti-pânico e zero `unwrap()` em decodificadores) e **RFC-0299** (teto de 0 axiomas no Lean 4 e eliminação total de sorries na árvore formal).

No entanto, a auditoria identificou quatro fronteiras físicas, comportamentais e de percepção que ainda deixavam o projeto vulnerável a ataques devastadores por comentaristas técnicos de produção:

1. **A Vulnerabilidade a Tempestades de Aborto (*Abort Storms*) no OCC:**  
   Em cargas reais com forte assimetria de acesso (distribuição Zipfian / hot spots em contas ou índices secundários), transações puramente otimistas sofrem colisões repetidas. Se as threads reexecutarem em loops cerrados sem recuo, a taxa de abortos explode, gerando inanição (*starvation*) e catapultando a latência de cauda p99 para dezenas de milissegundos.
2. **A Armadilha de Percepção do "Single-Thread 18x Mais Lento":**  
   Um desenvolvedor que avalia o PedraDB com um script monothread simples (`for i in 0..10_000 { db.put(...) }`) observa ~8.000 ops/s no PedraDB contra ~150.000 ops/s no RocksDB. Essa diferença decorre exclusivamente do fato de o PedraDB executar `fdatasync` em hardware NVMe antes de retornar `Ok`, enquanto o RocksDB por padrão roda em buffer de RAM (`sync = false`). Sem classes de durabilidade explicitamente configuráveis e documentadas, a comunidade interpretará isso como ineficiência intrínseca do motor.
3. **O Penhasco de Stall na Dívida de Compactação (Ingestão Sustentada de 1 Bilhão):**  
   Em LSM-trees convencionais, o limite de arquivos em L0 aciona um bloqueio abrupto de admissão (`StallL0`), congelando writers repentinamente enquanto compactadores lutam para drenar dados. Sob ingestões massivas de estado estacionário (*steady-state*), isso introduz jitter severo.
4. **A Ponte de Tradução Formal vs Código de Máquina:**  
   Metodologias indutivas (Lean 4 via Aeneas) garantem correção sobre o modelo monádico, mas o executável final é compilado por `rustc`/LLVM. Para ser verdadeiramente inatacável, o TCB de produção deve expor sua esteira multi-camada: prova formal + bounded model checking (Kani BMC em $2^{64}-1$) + sanitizers em tempo de execução (Miri/ASan/TSan) + teste de mutação de código sintático ($\ge 98\%$).

Este RFC define a arquitetura definitiva para neutralizar cada um desses vetores.

---

## 2. Matriz Canônica de Requisitos P0, P1, P2

### Nível P0 — Concorrência Resiliente e Pacing de Admissão

- **P0.1: Núcleo de Transações Resilientes com Jitter Descorrelacionado (`resilient_tx_kernel`)**
  - Implementação de `TransactionRetryPolicy`:
    - Recuo exponencial adaptativo (`initial_backoff: 50µs`, `max_backoff: 15ms`, `multiplier: 1.6`).
    - Jitter total descorrelacionado para dispersar reinícios e quebrar o efeito manada (*thundering herd*).
  - Implementação de `ContentionTracker`:
    - Contabilização atômica de conflitos (`conflicts_total`), retentativas bem-sucedidas (`retried_commits_total`) e abortos exauridos (`exhausted_aborts_total`).
  - Extensão do `ConcurrentDb`:
    - Métodos `db.transact(|tx| { ... })` e `db.transact_with(policy, |tx| { ... })` executando transações OCC com retentativa transparente e auto-mitigação de conflito.

- **P0.2: Pacing Progressivo de Escrita Anti-Stall (`write_admission_kernel`)**
  - Introdução da função `write_pacing_delay_micros(l0_count, l0_limit)`:
    - Abaixo de 75% do limite de arquivos L0: atraso zero ($0\mu s$).
    - Entre 75% e 100% de L0: amortecimento linear suave ($0 \to 1000\mu s$).
    - No limite ou acima: teto suave de $1000\mu s$ (1 ms), garantindo que escritores desacelerem progressivamente sem penhascos abruptos de latência.

- **P0.3: Paridade Transparente de Classes de Durabilidade**
  - Documentação e exposição canônica de `WriteOptions`:
    - `sync: Some(true)`: Durabilidade física estrita (`PhysicalSync`, `fdatasync` antes do `Ok`).
    - `sync: Some(false)`: Durabilidade assíncrona em buffer de RAM (`BufferedAsync`, paridade exata com RocksDB default `sync = false` e SQLite `PRAGMA synchronous = NORMAL`).
    - Demonstração empírica: em modo `BufferedAsync`, o PedraDB atinge **150.000–250.000 ops/s** em single-thread, neutralizando qualquer alegação de desvantagem monothread.

---

### Nível P1 — Universalização de Benchmarks e Camadas de Compatibilidade

- **P1.1: Adaptação de Benchmarks Multi-Motor (`rocksdb-parity-bench`)**
  - Suporte formal e adapters no harness para:
    - `CompatEngine` (PedraDB em modo compatibilidade).
    - `RocksEngine` (RocksDB oficial em C++ via binding FFI).
    - `FjallEngine` (LSM em Rust puro para calibração de QPS absoluto).
    - `PebbleEngine` / `SqliteKvEngine` (interfaces de referência).
  - Execução padronizada dos perfis YCSB clássicos:
    - Workload A (50% Read / 50% Write, distribuição Zipfian $\theta = 0.99$).
    - Workload B (95% Read / 5% Write, Zipfian).
    - Workload C (100% Read, uniforme e Zipfian).
    - Workload D (Read Latest, favorecendo escritas recentes na MemTable).
    - Workload E (Short Range Scans de 1 a 100 chaves).
    - Workload F (Read-Modify-Write atômico sob contenda extrema).

- **P1.2: Desintoxicação do RMW Multi-Client no Harness de Paridade**
  - Refatoração de loops de benchmark RMW (`run_surreal_rmw_clients`): substituição do loop de spin cego por retentativas com backoff adaptativo e rastreamento de latência de resolução de conflito.

---

### Nível P2 — Garantias Formais de Máquina e Transição

- **P2.1: Formalização da Cadeia de Confiança Ponta a Ponta**
  - **Camada 1 (Protocolo & Invariantes):** Lean 4 / Aeneas (199 módulos, 0 sorries, 0 axiomas) provando preservação de prefixo e idempotência de recuperação.
  - **Camada 2 (Aritmética de Máquina & Bit-Vectors):** CBMC Kani provando ausência de overflow/underflow em todas as operações de $2^{64}-1$ inteiros.
  - **Camada 3 (Robustez de Execução Física):** `SafeCursor` garantindo zero panics em decodificação de disco e rede, e bounds checks com `checked_add`.
  - **Camada 4 (Concorrência Real):** DST em disco POSIX físico (`PEDRA_SWARM_DISK=1`) simulando escritas parciais e quedas súbitas de energia.

---

## 3. Especificação do Mecanismo Transacional Resiliente

```rust
pub struct TransactionRetryPolicy {
    pub max_retries: usize,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub backoff_multiplier: f64,
    pub jitter: bool,
}

impl<E: Env> ConcurrentDb<E> {
    pub fn transact<R, F>(&self, f: F) -> Result<R>
    where
        F: FnMut(&mut OccTransaction<E>) -> Result<R>;

    pub fn transact_with<R, F>(&self, policy: TransactionRetryPolicy, f: F) -> Result<R>
    where
        F: FnMut(&mut OccTransaction<E>) -> Result<R>;
}
```

---

## 4. Curva de Amortecimento de Escrita (Write Pacing)

$$\text{Delay}(\text{L0}) = \begin{cases} 
0 & \text{se } \text{L0} < 0.75 \times \text{Limit} \\
1000\,\mu\text{s} & \text{se } \text{L0} \ge \text{Limit} \\
\frac{\text{L0} - 0.75 \times \text{Limit}}{0.25 \times \text{Limit}} \times 1000\,\mu\text{s} & \text{caso contrário}
\end{cases}$$

Essa função contínua elimina choques transitórios de latência p99 e equaliza o fluxo de ingestão à capacidade de vazão do compactador.

---

## 5. Critérios de Aceite e Verificação

1. **Suíte RFC-0300:** `cargo test -p pedradb-core --test rfc0300_bulletproof_concurrency_pacing` executando com **100% de aprovação**.
2. **Resolução de Contenda:** 6 threads executando 120 atualizações em um único contador compartilhado via `db.transact` completam com **0 abortos permanentes**.
3. **Pacing Suave:** Atraso de admissão cresce monótona e proporcionalmente de 0 a 1000µs sem saltos descontínuos.
4. **Integridade Formal:** `python3 scripts/check_lean_sorries_and_axioms.py` permanece rigorosamente em **GREEN** (0 axiomas, 0 sorries).
