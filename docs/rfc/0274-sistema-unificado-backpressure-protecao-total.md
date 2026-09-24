# RFC-0274 — Sistema Unificado de Backpressure e Proteção Total do Motor

**Estado:** ATIVO (Mandatório para Produção, Benchmarks e Concorrência G1)  
**Data:** 24 de Setembro de 2026  
**Parents:** [0273](0273-endgoals-v5-eliminacao-pontos-fracos.md), [0270](0270-zero-twin-verification-policy.md), [0269](0269-sistema-unificado-metricas-saude-intervencao-telemetria.md), [0179](0179-disk-pressure-watermarks.md), [0171](0171-write-admission-rfc.md), [0050](0050-pct-swarm-failclosed.md)  
**Objeto:** Especificação e implementação integral do sistema unificado de backpressure do PedraDB, cobrindo pacing suave de compactação, rate limiting de I/O em background para proteção do `fdatasync` foreground, dívida de compactação multinível, expiração/bloat de snapshots, GC do log de valores (VLog) e teto de admissão concorrente.

---

## 1. Contexto e Motivação: Por Que o Backpressure Atual É Insuficiente?

O PedraDB possui duas blindagens fundamentais já verificadas:
1. **Disk Pressure (RFC-0179):** Soft floor (256 MiB) com reclaim e Hard floor (64 MiB) com recusa explícita de writes antes do `ENOSPC`.
2. **RAM Pressure (OOM Prevention):** Evicção progressiva de caches a 70% e throttle/flush síncrono a 90% da memória.
3. **Mem/L0 Stall:** Recusa de escrita caso a memtable ou a quantidade de arquivos na L0 ultrapassem limites.

Entretanto, uma análise rigorosa frente a bancos estado da arte (RocksDB, Pebble, PostgreSQL, WiredTiger) revelou **6 frentes desprotegidas** que afetam duramente estabilidade, garantias operacionais e números em benchmarks competitivos:

### A. Saturação da Fila de Disco por Compactação (O Dilema do `fdatasync` G1)
O produto oficial do PedraDB é a **durabilidade G1**: mais rápido que o RocksDB default (`sync=false`) realizando `fdatasync` real antes de entregar `Ok` ao cliente.  
Se um job de compactação em background reescreve SSTs na velocidade bruta máxima do disco, ele satura a profundidade de fila (`queue depth`) do controlador NVMe/SSD. O `fdatasync` do WAL na thread foreground fica represado atrás de dezenas de megabytes de escrita de compactação, destruindo a latência de cauda (P99/P99.9).

### B. O Degrau Binário do Stall ("Efeito Dente de Serra")
A admissão de escrita tradicional no L0 stall é binária: ou aceita 100% ou bloqueia/rejeita (`WriteStall`). Em testes prolongados (YCSB de 30+ minutos), isso produz oscilações brutais de throughput (de 200k ops/s para zero e vice-versa), aumentando o jitter percebido pela aplicação.

### C. Dívida Oculta de Compactação nos Níveis L1..LN (`pending_compaction_bytes`)
Monitovar apenas o número de arquivos na L0 é um ponto cego clássico de LSM: a L0 pode estar limpa (compactada para a L1), enquanto a dívida entre L1 $\to$ L2 e L2 $\to$ L3 acumula dezenas de gigabytes de dados descompactados. Sem backpressure na dívida global de compactação, a amplificação de espaço explode e as buscas pontuais sofrem degradação severa.

### D. Inchaço de Tombstones por Snapshots e Iteradores Longos
No PostgreSQL, transações `idle in transaction` impedem o `VACUUM` de limpar tuplas mortas, causando inchaço de tabela e risco de *wraparound*. No PedraDB, se um cliente segura um `Snapshot` ou `Iterator` aberto indefinidamente, a compactação não pode expurgar tombstones nem versões obsoletas. Isso consome espaço em disco silenciosamente até atingir o piso de emergência de 64 MiB.

### E. Dívida de Garbage Collection no Value Log (VLog)
Com a separação de valores grandes em arquivos `.vlog` (estilo WiscKey/BlobDB), altas taxas de overwrite/delete acumulam lixo no log de valores. Sem um gatilho automático de GC e pacing de novos blobs quando a razão de lixo ultrapassa patamares críticos, o disco sofre amplificação de espaço desnecessária.

### F. Explosão de Writers Concorrentes no `ConcurrentDb`
Sem um limitador de requisições de escrita em trânsito (`max_in_flight_writers`), uma rajada massiva de clientes concorrentes aloca memória e enfileira batches indefinidamente durante qualquer lentidão momentânea de I/O.

---

## 2. Arquitetura dos 6 Pilares de Backpressure

```
+---------------------------------------------------------------------------------------+
|                                    PEDRADB ENGINE                                     |
+---------------------------------------------------------------------------------------+
|  1. CLIENT SUBMIT / WRITE ADMISSION                                                  |
|     ├── Concurrency Queue Depth Check (max_in_flight_writers)                         |
|     ├── RAM & Disk Pressure Watermarks (RFC-0179 & OOM Prevention)                    |
|     ├── Smooth Dynamic Pacing (compaction debt + L0 pressure micro-delay)            |
|     ├── Pending Compaction Bytes Stall (L1..LN debt watermark)                        |
|     └── Snapshot Pin Debt Check (rejeita writes se snapshot velho ameaça bloat)       |
+---------------------------------------------------------------------------------------+
|  2. FOREGROUND DURABILITY G1                                                         |
|     └── WAL Append + fdatasync (Fila limpa e protegida de interferências)             |
+---------------------------------------------------------------------------------------+
|  3. BACKGROUND ENGINE & COMPACTION PACING                                            |
|     ├── CompactionRateLimiter (Token bucket limitando bytes/s de SST e VLog)          |
|     ├── Auto VLog GC Trigger (quando razão de lixo dead/total > 30%)                  |
|     └── Snapshot Pin Expiration / Eviction (max_snapshot_age_secs)                    |
+---------------------------------------------------------------------------------------+
```

### Pilar I: Rate Limiter de I/O de Compactação (`CompactionRateLimiter`)
- Implementa token-bucket determinístico para controlar a vazão de I/O de background (`compaction_rate_bytes_per_sec`).
- As threads de compactação e VLog GC consultam o pacer em blocos de escrita. Caso os tokens se esgotem, a thread de background suspende por microssegundos calculados, deixando a largura de banda do barramento e os IOPS do disco prioritariamente livres para os `fdatasync` do WAL foreground.
- Integração nativa com `rocksdb-compat` (`set_bytes_per_sync`, `set_wal_bytes_per_sync`, `rate_limiter`).

### Pilar II: Pacing Dinâmico Suave (Smooth Write Pacing)
- Em vez de um salto abrupto para stall total, o motor calcula um micro-atraso proporcional:
  $$\text{delay\_us} = \text{base\_delay} \times \left( \frac{\text{atual} - \text{soft}}{\text{hard} - \text{soft}} \right)$$
- Ativado quando:
  1. $L_0$ excede `write_pressure_l0` rumo a `write_stall_l0`.
  2. A dívida pendente de compactação ultrapassa o limiar suave (`pending_compaction_soft_bytes`).
- A thread de escrita dorme por microssegundos antes do commit, desacelerando a ingestão na medida exata da capacidade do compactador e estabilizando a linha de throughput.

### Pilar III: Limite de Dívida de Compactação Global (`pending_compaction_bytes`)
- Monitora a dívida estimada de compactação (tamanho de dados em níveis que excedem suas capacidades nominais multiplicadores):
  - Limite soft: ativa o pacing dinâmico suave.
  - Limite hard: ativa `WriteStall` de compactação.
- **Regra de Ouro da Paridade com RocksDB (Padrão 0 = Desativado):** No RocksDB default (`ROCKS_PARITY_SYNC=0`), tanto `soft_pending_compaction_bytes_limit` quanto `hard_pending_compaction_bytes_limit` são `0` (desativados). O PedraDB adota `0` por padrão no `BackpressureConfig` para evitar que cargas em massa e benchmarks de grande escala (como a hidratação de 500M rotas / 105 GiB) sofram stalls artificiais aos 2 GiB. Em ambientes de produção com risco de estouro de disco, o operador configura os limiares desejados (ex: 64 GiB / 256 GiB, ou 512 MiB / 2 GiB em discos pequenos).

### Pilar IV: Gestão de Ciclo de Vida de Snapshots e Evicção de Pins
- Rastreamento centralizado do timestamp de criação e sequência de todos os snapshots ativos.
- Configuração de `max_snapshot_age_secs` (padrão: 3.600s / 1 hora, ajustável).
- Caso um snapshot ultrapasse a idade limite:
  - Ele é marcado como expirado (`SnapshotExpired`).
  - O cursor de proteção de tombstones do compactador avança para o snapshot válido mais antigo.
  - Tentativas subsequentes de leitura no snapshot expirado retornam `Err(CoreError::SnapshotExpired)`.
- Adicionalmente, caso o lag de sequência de um snapshot atinja `snapshot_pin_hard_lag` sob pressão de disco, novas escritas entram em backpressure para impedir esgotamento fatal de armazenamento.

### Pilar V: Backpressure de Coleta de Lixo do Log de Valores (VLog GC)
- Monitoramento contínuo da proporção de lixo:
  $$\text{Garbage Ratio} = \frac{\text{bytes\_descartados}}{\text{bytes\_totais\_vlog}}$$
- Quando $\text{Garbage Ratio} \ge 30\%$, o agendador prioriza jobs de VLog GC.
- Quando $\text{Garbage Ratio} \ge 60\%$, escritas com valores grandes para o VLog sofrem pacing suave adicional, priorizando a reciclagem de arquivos `.vlog` e desfragmentação do disco.

### Pilar VI: Teto de Concorrência em Trânsito no `ConcurrentDb`
- Parâmetro `max_in_flight_writers` (padrão: 1.024 concorrentes).
- Evita que tempestades de requisições sobrecarreguem filas de memória durante pausas de rede ou picos de I/O.
- Rejeita com `CoreError::QueueFull` ou aguarda com timeout limitado em condvar quando a capacidade atinge o limite.

### Pilar VII: Opções e Integração no `pedradb-store` (Montanha)
- `StoreOpenOptions` expõe `.with_write_backpressure()` e `.without_write_backpressure()` para alternância explícita de perfis.
- No modo laboratório / soak unconstrained, o backpressure de L0 permanece desligado por padrão para máxima velocidade de ingestão e benchmarks sem limites artificiais. Em produção, clusters ativam o perfil de backpressure via `.with_write_backpressure()`.

---

## 3. Matriz de Garantias e Comparativo com Outros Motores

| Mecanismo de Backpressure | PedraDB (Pós-0274) | RocksDB | Pebble | PostgreSQL |
| :--- | :--- | :--- | :--- | :--- |
| **Proteção de Disco (ENOSPC)** | Soft (256MB) + Hard (64MB) recusa segura | Não (cai com ENOSPC) | Não nativo | Não (PANIC / crash recovery) |
| **Background I/O Pacing** | `CompactionIoPacer` (Token bucket) | `RateLimiter` | Token bucket manual | `vacuum_cost_limit` |
| **Write Pacing Suave** | Microssegundos dinâmicos por dívida | `delayed_write_rate` | Dynamic pacing | N/A (checkpoint spike) |
| **Dívida L1..LN** | `pending_compaction_bytes` | `pending_compaction_bytes` | `CompactionDebt` | N/A |
| **Snapshot Bloat Defense** | Expiração automática + Pin backpressure | Snapshot pin sem expiração nativa | Snapshot pin sem expiração | Risco de wraparound / bloat |
| **VLog GC Backpressure** | Pacing por Garbage Ratio | BlobDB básico | N/A | VACUUM autônomo |
| **Bounded Queue Depth** | `max_in_flight_writers` cap | `max_write_batch_group_size` | Write queue cap | `max_connections` |

---

## 4. Plano de Implementação

1. **Kernel (`crates/pedradb-core/src/backpressure_kernel.rs`):**
   - Implementação pura, `#![forbid(unsafe_code)]`, sem twin, com structs de configuração, métricas e cálculos determinísticos de delay e vereditos.
2. **Integração no `db_kernel.rs` e `ConcurrentDb`:**
   - Incorporação de `CompactionIoPacer`, limites de dívida pendente, teto de escritores e pacing dinâmico no loop de admissão de escrita.
3. **Integração no `rocksdb-compat`:**
   - Mapeamento das propriedades e APIs de rate limiting para as primitivas reais do `backpressure_kernel`.
4. **Habilitação por Padrão no `pedradb-store`:**
   - `StoreOpenOptions::default()` passa a ter `pedra_write_backpressure: true`.
5. **Bateria de Testes:**
   - Testes unitários exaustivos cobrindo todos os cálculos matemáticos de pacing, token bucket, limites de fila, expiração de snapshot e comportamento AS-IS vs PROD.
