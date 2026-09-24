# Relatório de Pesquisa e Arquitetura: Observabilidade Interna, Diagnóstico de Saúde e Intervenção Operacional

**Data:** 2026-09-24  
**Autores:** Engenharia de Sistemas & Pesquisa PedraDB  
**Status:** IMPLEMENTADO (P0/P1 entregues em `pedradb-core`, `pedradb-apply` e `pedradb-http`)  
**RFC Associado:** [RFC-0269](../rfc/0269-sistema-unificado-metricas-saude-intervencao-telemetria.md)  
**Ledger:** Entradas L59 a L63 em `research/LEDGER.md`  

---

## 1. Visão Executiva e Problema Operacional

Em bancos de dados de armazenamento chave-valor embarcados ou distribuídos, a observabilidade tradicional sofre de uma patologia crônica: **excesso de contadores passivos desprovidos de diagnóstico acionável**. Motores como RocksDB expõem centenas de contadores e propriedades (`rocksdb.num-running-flushes`, `rocksdb.estimate-pending-compaction-bytes`, etc.), mas transferem inteiramente ao operador humano (ou a scripts frágeis de monitoramento externo) a responsabilidade de inferir:
1. *O banco está saudável ou degradado?*
2. *Os clientes estão prestes a sofrer um write stall catastrófico?*
3. *Existe vazamento de snapshot (pin leak) impedindo a coleta de lixo MVCC e inflando o disco?*
4. *Que ação de intervenção manual ou automatizada deve ser tomada agora?*

Esta lacuna gera incidentes graves em produção: discos cheios por pins esquecidos, cauda de latência (p99/p999) multiplicada por 100× sob write stalls não sinalizados, e compações silenciosamente falhando até o esgotamento do sistema de arquivos.

Este trabalho estabelece a arquitetura de **Observabilidade Ativa e Diagnóstico de Saúde de Primeira Classe** para o PedraDB, baseando-se em pesquisa comparativa exaustiva com os principais sistemas da literatura e da indústria.

---

## 2. Levantamento Comparativo da Literatura e Motores

Para estruturar a telemetria do PedraDB, realizamos um benchmarking conceitual sobre o estado da arte em diferentes classes de sistemas de banco de dados:

### 2.1 Motores LSM Embarcados (Embedded KV)

| Motor | Métricas Fornecidas | Mecanismo de Alerta / Eventos | Lacuna / Falha Identificada |
|---|---|---|---|
| **RocksDB** | `Statistics` (tickers, histos), `DBProperties` (40+ strings), `CompactionStats` | `EventListener` (`OnStallConditionsChanged`, `OnErrorRecoveryBegin`, `OnFlushCompleted`) | **Passivo e não interpretado:** RocksDB não emite um veredito consolidado de saúde. O operador precisa configurar 30 regras no Prometheus para descobrir se a compactação está endividada. |
| **Pebble** | `Metrics` struct (memtable, lsm, disk space, compact debt), `EventListener` | Callbacks de stall e write delay | Métricas agrupadas por tipo, mas carece de diagnóstico acionável embutido de remediação. |
| **Fjall** | Partition stats, segment counts, active memtable size, block cache usage | `flush_sem` (bloqueio atômico de writers) | Não possui diagnóstico formal de saúde ou recomendações de intervenção emitidas pelo motor. |

### 2.2 Motores Distribuídos e Documentais

| Motor | Métricas Fornecidas | Mecanismo de Alerta / Eventos | Lacuna / Falha Identificada |
|---|---|---|---|
| **FoundationDB (FDB)** | `status json` (árvore hierárquica completa), `TraceEvent` estruturado, métricas de `processes`, `storage_servers`, `recovery_state` | `cluster.messages` (alertas explícitos: `unreachable_process`, `low_disk_space`, `moving_data`) | **Padrão ouro em JSON estruturado**. Contudo, o cliente em C++ requer runtime Flow pesado e não se adapta diretamente a bibliotecas embarcadas leves em Rust. |
| **MongoDB / WiredTiger** | `serverStatus`: `wiredTiger.cache` (dirty bytes, eviction by app threads), `concurrentTransactions` (read/write tickets) | Oplog window metric, dirty cache watermark thresholds | Quando threads de aplicação realizam desalocação forçada (`app thread eviction`), a latência explode sem aviso prévio. |
| **CockroachDB** | Node liveness lease, unresolved intents debt, range quiescence, raft ready latency | `Health` status endpoint, liveness heartbeat | Forte em métricas de consenso distribuído, mas focado na camada SQL/Raft e não no kernel LSM local. |

### 2.3 Bancos Relacionais Tradicionais (RDBMS)

| Motor | Métricas Fornecidas | Mecanismo de Alerta / Eventos | Paralelo com o PedraDB |
|---|---|---|---|
| **PostgreSQL** | `pg_stat_activity` (wait events), `pg_stat_database`, `pg_stat_bgwriter`, `pg_stat_wal`, `pg_stat_progress_vacuum` | Alertas de transaction ID wraparound (`txid_current() - datfrozenxid > 200M`) | **Idêntico ao Snapshot Pin Leak no PedraDB:** se uma transação aberta esquecer de comitar/fechar, o autovacuum não consegue recolher dead tuples; no PedraDB, um `SnapshotPin` antigo trava `earliest_readable_seq`, impedindo a liberação de tombstones e memtables. |

---

## 3. Inovações Modernas em Telemetria (2024–2026)

Pesquisamos as tendências recentes de engenharia de confiabilidade de dados (SRE / Observability):
1. **Tri-State Operational Health Model:** Superação do modelo binário (Up/Down). O motor opera em `Healthy` (normal), `Degraded` (pressão suave, cache thrashing, dead space elevado, mas sem bloqueio) ou `ActionRequired` (write stall ativo, falhas repetidas de compactação, pin leak crítico, corrupção).
2. **Actionable Remediation Advice (Diagnóstico Prescritivo):** Em vez de alertar apenas `l0_files = 16`, o motor emite a recomendação exata: `RunManualCompaction { reason: "L0 files exceeded limit" }` ou `ReleaseLeakedSnapshots { oldest_seq: 1024, lag: 35000 }`.
3. **Zero-Lock Atomic Telemetry:** Amostragem de métricas que nunca adquire locks exclusivos de escrita (`parking_lot::RwLock` de leitura rápida ou carregamentos atômicos `Relaxed`/`Acquire`), impedindo que ferramentas de monitoramento causem jitter de I/O na thread crítica de commit.
4. **Multi-Protocol Telemetry:** Suporte simultâneo nativo a exportação no formato de texto padrão Prometheus/OpenMetrics (sem puxar frameworks de telemetria de 20 MB como dependência) e formato JSON hierárquico estruturado (estilo FoundationDB `status json`).

---

## 4. Arquitetura Implementada no PedraDB

### 4.1 Módulo Puro de Saúde (`crates/pedradb-core/src/health_kernel.rs`)
Implementado com `#![forbid(unsafe_code)]`, determinístico e livre de I/O oculto:
- `HealthStatus`: Enum derivando `Ord`, `PartialOrd`, `Eq`, `PartialEq`, `Debug`:
  - `Healthy = 0`
  - `Degraded = 1`
  - `ActionRequired = 2`
- `HealthIssue`: Diagnóstico específico com parâmetros numéricos:
  - `WriteStalledL0`: Limite L0 excedido.
  - `WriteStalledMem`: Limite de memória de memtable excedido.
  - `WritePressureL0`: Alerta de pressão suave de escrita.
  - `SnapshotPinLeak`: Alerta de pin MVCC antigo retendo versões.
  - `AutoCompactFailing`: Contagem e erro da última falha de compactação em segundo plano.
  - `VlogFragmentationHigh`: Fragmentação e bytes mortos no log de valores (WiscKey/blob).
  - `DiskSpaceLow` / `DiskSpaceCritical`: Detecção proativa de escassez de disco baseada em watermarks do RFC-0179.
  - `CorruptionDetected`: Eventos de integridade/CRC registrados no `CORRUPTLOG`.
  - `BlockCacheThrashing` / `TableCacheThrashing`: Taxa de acerto abaixo de 50% sob alta carga.
- `RecommendedIntervention`: Enum com ações específicas (`RunManualCompaction`, `ReleaseLeakedSnapshots`, `RunVlogGc`, `IncreaseBlockCacheCapacity`, `CheckDiskAndPermissions`, `IsolateAndRestoreFromBackup`, `ProvisionMoreDiskSpace`, `NoActionRequired`).
- `evaluate_db_health`: Função pura que aceita `DbStats`, watermarks de pressão de disco e contagem de corrupções para gerar um `DbHealthReport`.
- `format_prometheus_metrics`: Gerador nativo de métricas OpenMetrics com linhas `# HELP` e `# TYPE`.
- `format_json_status`: Gerador de JSON estruturado contendo blocos `health`, `storage`, `lsm`, `mvcc` e `cache`.

### 4.2 Expansão do `DbStats` no Kernel do Motor (`crates/pedradb-core/src/db_kernel.rs`)
Campos adicionados e mantidos compatíveis com `Eq`:
- `oldest_pin_seq: Option<SequenceNumber>`
- `pin_seq_lag: u64`
- `total_disk_bytes: u64` (SST + WAL + Vlog)
- `is_write_stalled: bool`
- `is_write_pressured: bool`
- `corruptions_detected: u64`
- Métodos calculados em tempo constante: `table_cache_hit_ratio()`, `block_cache_hit_ratio()`, `write_amplification()`, `space_amplification()`.

### 4.3 Exposição no Façade de Aplicação (`pedradb-apply`)
- `KvService::stats(&self) -> DbStats`
- `KvService::health(&self) -> DbHealthReport`

### 4.4 Exposição HTTP, Telemetria e Webhooks (`pedradb-http`)
- `GET /metrics`: Retorna texto no padrão Prometheus (`text/plain; version=0.0.4; charset=utf-8`), permitindo scraping instantâneo por Grafana Agent, Prometheus, Datadog ou OpenTelemetry Collector.
- `GET /health` e `GET /status`: Retorna JSON estruturado. **Comportamento Cloud-Native Fail-Closed:**
  - Status `Healthy` ou `Degraded` → HTTP 200 OK.
  - Status `ActionRequired` → HTTP 503 Service Unavailable (desviando tráfego de load balancers Kubernetes/ALB imediatamente).
- `dispatch_health_webhook`: Despachador HTTP POST zero-dependency capaz de enviar notificações automáticas em JSON para webhooks do Slack, PagerDuty ou controladores de auto-remediação.

---

## 5. Ledger de Decisões Metodológicas

| ID | Decisão | Tag | src | Justificativa Técnica |
|---|---|---|---|---|
| **L59** | Tri-State Health (`Healthy`/`Degraded`/`ActionRequired`) + Diagnóstico Prescritivo | `SHIP` | code | Transforma contadores passivos em vereditos acionáveis pelo operador e por orquestradores de automação. |
| **L60** | Expositores Zero-Dependency (Prometheus Text + FoundationDB JSON) | `SHIP` | code | Elimina a necessidade de puxar SDKs de 20 MB (OpenTelemetry/Serde pesados) no kernel do motor, preservando portabilidade e compilação ultra-rápida. |
| **L61** | Auto-Tuning Dinâmico em Background (AIMD online) | `REFUSE` | ficha (R016) | Já rejeitado por L42: auto-tuners adaptativos em background introduzem não-determinismo hostil ao DST e imprevisibilidade de latência sob jitter de disco. |
| **L62** | Coleta de Métricas com Locks Exclusivos de Escrita | `REFUSE` | code | Amostragem de métricas não deve disputar lock de escrita com a thread crítica de commit; apenas locks de leitura ou atômicos lock-free são permitidos. |
| **L63** | Endpoints HTTP `/metrics`, `/health` e Webhook Dispatcher | `SHIP` | code | Integração imediata com infraestrutura moderna de SRE, Kubernetes probes (200 vs 503) e sistemas de alerta. |

---

## 6. Verificação e Evidências

Todos os novos componentes foram testados com suítes unitárias e de integração de ponta a ponta:
1. `health_kernel::tests::test_health_evaluation_healthy`: PASS
2. `health_kernel::tests::test_health_evaluation_write_stalled`: PASS
3. `health_kernel::tests::test_health_evaluation_pin_leak`: PASS
4. `health_kernel::tests::test_health_evaluation_corruption_escalates_highest`: PASS
5. `health_kernel::tests::test_prometheus_formatting_contains_core_metrics`: PASS
6. `health_kernel::tests::test_json_status_formatting_valid_structure`: PASS
7. `health_kernel::tests::test_live_db_health_and_stats` (abertura de `Db` real no disco, inserções, snapshot pin, verificação de lag e cálculo de amplificação): PASS
8. `pedradb-http::tests::kv_http_metrics_and_health_endpoints` (servidor HTTP real, endpoints `/metrics`, `/health` e `/status`): PASS

A suíte completa de `pedradb-core` e `pedradb-http` permanece 100% verde.
