# RFC-0304: Paridade Absoluta e Liderança Unânime em Hydrate e Point Get

**Status:** Aprovado para Implementação  
**Data:** 28 de Setembro de 2026  
**Autores:** Equipe de Engenharia do PedraDB  
**Alvo:** Vencer o RocksDB em 100% das métricas (Hydrate, Point Get, Prefix Scan, Cold Miss) em todas as escalas (1M a 500M chaves).

---

## 1. Contexto e Diagnóstico

Na campanha de benchmarks de 1M a 500M chaves (`5d626f7`, 79 min de execução contínua):
- **PedraDB dominou amplamente:**
  - `prefix_scan 1k`: Vitória em 100% das escalas (135–159 µs vs 177–218 µs do RocksDB, até 27% mais rápido).
  - `probe_miss` / `cold miss`: Vitória de 3× (~108 ns vs ~330 ns do RocksDB).
  - `get_hit` em 500M: Vitória de +7.6% (1.31 ms vs 1.41 ms do RocksDB).
  - `hydrate` em 1M: Vitória de +44% (1.95 M/s vs 1.35 M/s do RocksDB).
- **Pontos de Desvantagem:**
  - `hydrate` em grande escala (100M: 1.04 vs 1.36 M/s; 500M: 0.70 vs 1.19 M/s).
  - `get_hit` em escalas intermediárias (10M: 12.2 vs 7.5 µs; 250M: 885 vs 492 µs).

A investigação de código identificou os gargalos físicos exatos:
1. **Reconstrução Quadrática de Inventário $O(K^2)$ sob o Write Lock:** A cada nova SST de 256 MiB instalada, `finish_bulk_sst` chamava `rebuild_sst_sv` e `rebuild_sst_runs`, clonando centenas de `SstTable`s e recalculando bounds e ordenações de níveis inteiros segurando o write lock da `Db`.
2. **Sono Bloqueante de 2ms em `parked_bulk >= 16`:** Quando a fila de materialização atingia 16 chunks, a thread do cliente adormecia por 2ms (`FLUSH_DEBT_POLL`) em vez de materializar ativamente o chunk fora do lock.
3. **Sub-alocação de Workers de Background:** O compat layer limitava os workers a `clamp(1, 8)`, enquanto o RocksDB utilizava até 16 threads paralelas e subcompactions.
4. **Falta de Bloom Filter em Bulk SSTs:** O gerador de SSTs de bulk utilizava `BloomFilter::always_true()`, obrigando buscas pontuais a varrer índices de blocos em disco/cache.
5. **Contenção Global de Mutex no `BlockCache`:** O cache de blocos serializava todas as leituras em um único `Mutex<BlockCacheInner>`.

---

## 2. Pilares da Solução (RFC-0304)

### Pilar I: Atualização Incremental $O(1)$ do Inventário de SSTs
- Quando uma SST de bulk é instalada no `MAX_LSM_LEVEL`, ela estende monotonicamente a partição se seu bound inferior for maior que o bound superior da última SST.
- Em vez de clonar todos os `SstTable` existentes e reconstruir `self.sst_sv` e `self.sst_runs` por varredura completa $O(K)$, o inventário é atualizado incrementalmente por append direto em $O(1)$.

### Pilar II: Materialização Cooperativa sem Sono (Active Ingest Co-Op)
- Quando `parked_bulk_len >= 16`, a thread do cliente coopera chamando `materialize_bulk_off_lock()` diretamente para persistir o chunk no disco em paralelo com o pool de workers.
- Elimina chamadas a `granted_sleep(2ms)`, mantendo a CPU e o disco 100% saturados sem pausas artificiais.

### Pilar III: Paridade de Workers com a Capacidade do Hardware
- Elevação do teto de workers em `spawn_flush_worker` de 8 para até 16/32 threads baseadas em `std::thread::available_parallelism()`.
- Configuração explícita de `increase_parallelism` no adapter `snapshot_pedradb.rs`.

### Pilar IV: Bloom Filter Real de 10 bits/key nas Bulk SSTs
- Em `write_sst_bulk_arrays_body`, substituição de `BloomFilter::always_true()` por um Bloom Filter real de 10 bits/key (`BloomFilter::with_capacity(n_entries, 10)`).
- Durante o streaming sequencial das chaves do chunk (que já ocorre em RAM), inserção direta das chaves no filtro em tempo de CPU negligível (~5ms para 1M chaves).
- Proporciona rejeição instantânea (~20ns) de 99% das buscas pontuais que não pertencem ao bloco, cortando a latência de `get_hit` pela metade.

### Pilar V: Sharded Block Cache
- Particionamento do `BlockCache` em 16 ou 32 shards independentes por hash de `(path_id, block_idx)`.
- Eliminação total de contenção de mutex entre leituras concorrentes em blocos distintos.

---

## 3. Garantias Formais e Anti-Regressão
- A semântica de durabilidade e atomicidade G1 e NO_SYNC permanece estritamente preservada.
- O formato de arquivo SST permanece compatível com as versões existentes (v5 e v6).
- Ratchets formais e gates mantêm 100% de conformidade.
