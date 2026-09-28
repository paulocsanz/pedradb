# RFC-0293: Hegemonia Universal em Todos os Benchmarks: Hydrate Concorrente Paralelo, Eliminação de Alocações em Compat Layer e Coalescência de Multi-Get

- **Status:** Proposto e Implementado
- **Data:** 2026-09-28
- **Autores:** PedraDB Core & Performance Engineering Team
- **Escopo:** `pedradb-core`, `rocksdb-compat`, `snapshot-bench`
- **Metas Rigorosas:**
  1. **Superar RocksDB em 100M Hydrate:** < 55s (> 1.80 M/s vs 1.76 M/s RocksDB).
  2. **Superar RocksDB e Fjall em 100M Lookup_100:** < 1.00 ms (get_loop e multi_get vs 1.40 ms RocksDB e 1.10 ms Fjall).
  3. **Superar RocksDB e Fjall em 100M get_hit:** < 10.0 µs vs 14.55 µs RocksDB e 10.45 µs Fjall.
  4. **Consolidar Vitória Total em 500M:** Manter a vitória hegemônica em todas as métricas de leitura (Cold miss, Cold hit, probe_miss, get_hit, prefix_scan, lookup_100) e acelerar Hydrate para < 270s (> 1.85 M/s).

---

## 1. Diagnóstico Físico e Limitações Estruturais Anteriores

Na medição do commit `1ad71a61` em 500M e 100M, o PedraDB provou superioridade hegemônica em escala de 500M:
- Cold miss: **135 ns** vs 383 ns RocksDB (**2,84× mais rápido**)
- probe_miss: **105 ns** vs 358 ns RocksDB (**3,41× mais rápido**)
- Cold hit: **48,4 µs** vs 69,9 µs RocksDB (**1,44× mais rápido**)
- get_hit: **38,89 µs** vs 66,33 µs RocksDB (**1,71× mais rápido**)
- prefix_scan 1k: **152,20 µs** vs 174,12 µs RocksDB (**1,14× mais rápido**)
- lookup_100: **3,72 ms** vs 5,63 ms RocksDB (**1,51× mais rápido**)

No entanto, duas fraquezas residuais persistiam em 100M e no throughput de hydrate:
1. **Sobrecarga de Alocação e Bloqueio em `get_cf_opt` e `multi_get_cf`:**
   - Em `rocksdb-compat`, enquanto `DB::get` utilizava `LAST_GET` TLS cache, buffer thread-local `encode_with` e o caminho lock-free `ConcurrentDb::get` (que possui bypass rápido de outside-SST e block cache sem mutex de Db), o método `DB::get_cf_opt` redirecionava para `get_at`.
   - `get_at` alocava uma nova `Vec<u8>` no heap via `self.codec.encode(cf, key)` para cada chamada, adquiria o `RwLock` de leitura do `Db` inteiro e não consultava os filtros lock-free.
   - Em `multi_get_cf`, a compat layer alocava múltiplos vetores intermediários (`Vec<(&ColumnFamily, K)>`, `Vec<Vec<u8>>`), consumindo centenas de alocações de heap por batch de 100 chaves.
2. **Gargalo Monotarefa no Dreno de Chunks Bulk (Hydrate Bottleneck):**
   - O thread de ingestão produzia lotes de 256 MiB em memória (`bulk_runs`).
   - O dreno em background contava com um único thread flusher com sleep de polling (`5 ms`) e capacidade limitada de chunks estacionados (`parked_bulk` limitado a 8 chunks = 2 GiB).
   - Quando o buffer enchia, a thread do escritor era obrigada a serializar e codificar o SST de 256 MiB síncronamente (`install_bulk_run`), interrompendo a ingestão contínua.

---

## 2. Inovações Estruturais e Solução

### 2.1 Fast-Path Zero-Allocation em `get_cf_opt` e `batched_multi_get_cf`
- Quando `ReadOptions.snap.is_none()`, `get_cf_opt` unifica-se ao pipeline de alta performance:
  1. Consulta a thread-local cache `LAST_GET` com `(epoch, gen, key)`;
  2. Executa a codificação de chave através do scratchpad TLS `self.codec.encode_with(&cf.name, key, ...)`, sem alocar qualquer byte no heap;
  3. Despacha para `self.inner.get(enc)`, alavancando o filtro de limites `fast_outside_sst_miss` e a consulta concorrente lockless.
- Implementação direta de `batched_multi_get_cf` na compat layer e paralelização direta em `PedraDbReader::multi_get`.

### 2.2 Desbloqueio e Aceleração do Pipeline de Hydrate
- Ampliação do runway de chunks estacionados de 8 para 16 chunks (4 GiB de amortecimento contínuo sem stalls síncronos).
- No loop do flusher em `rocksdb-compat`: detecção imediata de `has_parked_bulk()`, eliminando o sleep de 5 ms quando há chunks pendentes e drenando em loop contínuo sem latência.
- Separação estrita de locks: materialização de bulk chunks em segundo plano sem retenção concorrente desnecessária de locks de memtable.

---

## 3. Verificação Formal e Validação de Não-Regressão
- Todos os testes de conformidade de durabilidade (`rocksdb-compat` e `pedradb-core`) são mantidos 100% íntegros.
- Invariante `#![forbid(unsafe_code)]` preservado estritamente em todo o código Rust.
