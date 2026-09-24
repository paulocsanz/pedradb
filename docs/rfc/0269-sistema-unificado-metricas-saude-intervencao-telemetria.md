# RFC-0269: Sistema Unificado de Métricas Internas, Diagnóstico de Saúde, Intervenção Operacional e Telemetria Multi-Protocolo

- **Status:** Approved for Implementation (P0, P1, P2 Entregues)
- **Data:** 2026-09-24
- **Autor:** Paulo Cabral Sanz & Antigravity (Pair Programming)
- **Áreas Afetadas:** `pedradb-core`, `pedradb-apply`, `pedradb-http`, `research/LEDGER.md`
- **Peer/Bench:** RocksDB `Statistics`/`DBProperties`, FoundationDB `status json`, PostgreSQL `pg_stat_*`, MongoDB `serverStatus`

---

## 1. Contexto e Motivação

Métricas em motores de banco de dados tradicionalmente servem a um propósito passivo: exportar milhares de contadores atômicos não interpretados. Motores como RocksDB e Fjall acumulam tickers para hits de cache, arquivos de SST e contadores de compactação, mas deixam para o operador a tarefa manual de correlacionar esses números para determinar se o sistema está em colapso.

Em incidentes reais de produção, operadores enfrentam perguntas críticas que contadores brutos não respondem:
1. **O motor está aceitando gravações com latência normal ou está em *Write Stall*?**
2. **Existe um leitor MVCC retendo uma transação antiga (*Snapshot Pin Leak*), impedindo a coleta de lixo e consumindo o disco?**
3. **As compactações em segundo plano estão falhando silenciosamente por erro de I/O ou permissão?**
4. **O log de valores (WiscKey/blob) está com mais de 50% de espaço morto por deleções/updates?**
5. **Qual é a ação corretiva prescritiva imediata?**

Este RFC formaliza a arquitetura de **Diagnóstico Prescritivo e Telemetria Ativa de Primeira Classe** do PedraDB, inspirando-se no modelo de `status json` do FoundationDB, nos alertas de congelamento/wraparound do PostgreSQL e nos semáforos de write stall do RocksDB/WiredTiger, sem introduzir dependências pesadas nem quebrar o determinismo estrito (DST).

---

## 2. Desenho Arquitetural

```
+-------------------------------------------------------------------------------------------------+
|                                 RFC-0269 ARQUITETURA DE SAÚDE                                   |
+-------------------------------------------------------------------------------------------------+
|                                                                                                 |
|   [NÚCLEO ESTATÍSTICO] (crates/pedradb-core/src/db_kernel.rs)                                    |
|   - DbStats: contadores lock-free (L0, pin lag, total disk, WA, SA, cache hit ratios)           |
|                                                                                                 |
|   [KERNEL DE DIAGNÓSTICO PURO] (crates/pedradb-core/src/health_kernel.rs)                        |
|   - evaluate_db_health(stats, disk_admit, corruptions, config) -> DbHealthReport                |
|   - Tri-State Health: Healthy (0), Degraded (1), ActionRequired (2)                             |
|   - HealthIssue: WriteStalled, SnapshotPinLeak, AutoCompactFailing, VlogFragmentation, etc.     |
|   - RecommendedIntervention: RunManualCompaction, ReleaseLeakedSnapshots, RunVlogGc, etc.        |
|                                                                                                 |
|   [EXPOSITORES MULTI-PROTOCOLO NATIVOS]                                                         |
|   - format_prometheus_metrics(): OpenMetrics / Prometheus exposition text (# HELP, # TYPE)       |
|   - format_json_status(): Hierarchical FoundationDB-style status JSON                           |
|                                                                                                 |
|   [EXPOSIÇÃO EM CAMADAS & REDE]                                                                 |
|   - pedradb-apply::KvService: stats() e health() diretos                                        |
|   - pedradb-http:                                                                               |
|     * GET /metrics -> 200 OK text/plain (Prometheus scraper)                                    |
|     * GET /health, /status -> 200 OK (Healthy/Degraded) / 503 Service Unavailable (K8s probe)    |
|     * dispatch_health_webhook(): Push de alertas JSON para Slack/PagerDuty/Automated Remediation |
+-------------------------------------------------------------------------------------------------+
```

---

## 3. Especificação das Fases

### Fase P0: Métricas Expandidas e Modelo Tri-State Puro (`pedradb-core`)
- Implementar `crates/pedradb-core/src/health_kernel.rs` com `#![forbid(unsafe_code)]`.
- Estruturar os tipos:
  - `HealthStatus`: `Healthy = 0`, `Degraded = 1`, `ActionRequired = 2`.
  - `HealthIssue`: Diagnóstico específico com dados numéricos de contexto.
  - `RecommendedIntervention`: Ação recomendada direta para operadores humanos ou agentes autônomos.
  - `DbHealthReport`: Agregado imutável do veredito e recomendações.
- Expandir `DbStats` em `db_kernel.rs` para capturar:
  - `oldest_pin_seq`: Sequência do pin mais antigo ativo.
  - `pin_seq_lag`: Distância entre a última sequência e o pin mais antigo (`last_sequence - pin_seq`).
  - `total_disk_bytes`: Soma de SST, WAL e arquivos Blob/Vlog.
  - `is_write_stalled`: Indicador booleano instantâneo de write stall ativo.
  - `is_write_pressured`: Indicador de pressão suave de escrita.
  - `corruptions_detected`: Contagem de eventos no `CORRUPTLOG`.
  - Métodos em $O(1)$: `table_cache_hit_ratio()`, `block_cache_hit_ratio()`, `write_amplification()`, `space_amplification()`.

### Fase P1: Telemetria Multi-Protocolo Zero-Dependency
- `format_prometheus_metrics`: Formatação em tempo linear de texto compatível com Prometheus/OpenMetrics. Sem alocações desnecessárias, zero dependências externas.
- `format_json_status`: Estrutura JSON com blocos `{ "health": ..., "storage": ..., "lsm": ..., "mvcc": ..., "cache": ... }`, compatível com ferramentas modernas de ingestão (Elastic, Datadog, Grafana Loki, dashboards locais).
- Métodos integrados: `Db::health()`, `Db::health_with_config()`, `ConcurrentDb::health()`, `ConcurrentDb::health_with_config()`.

### Fase P2: Integração de Rede, Camada de Aplicação e Webhooks (`pedradb-apply` e `pedradb-http`)
- Exposição em `KvService`: métodos `stats()` e `health()`.
- Endpoints HTTP:
  - `GET /metrics`: Servido em formato OpenMetrics text.
  - `GET /health` e `GET /status`: Servido em formato JSON. Fail-closed: se o status for `ActionRequired`, responde com HTTP 503 Service Unavailable, orientando balanceadores a desviar tráfego imediatamente.
- Webhooks: Função auxiliar `dispatch_health_webhook(addr, endpoint, report, stats)` para envio de alertas ativos via HTTP POST.

---

## 4. O que foi Rejeitado e Por Quê (Decisões Negativas)

1. **Auto-Tuning Online via Threads em Background (AIMD de threads de compactação):**
   *Rejeitado (REFUSE L61/L42).* Motores que tentam ajustar dinamicamente tamanhos de batch ou contagem de threads de compactação com loops de feedback (ex.: ADOC FAST'23) degradam a previsibilidade do p99 e são hostis ao determinismo de teste (DST).
2. **Dependência de Frameworks Pesados de Telemetria no Kernel (`opentelemetry-sdk`, `prometheus-client`, `serde_json`):**
   *Rejeitado (REFUSE L60/L62).* O núcleo do PedraDB deve compilar em poucos segundos e rodar em qualquer ambiente (embarcado, WASM, seL4, Linux minimal). A formatação direta em buffer de strings para OpenMetrics e JSON atinge 100% dos requisitos sem inflar a árvore de dependências.
3. **Locks Exclusivos de Escrita para Coleta de Métricas:**
   *Rejeitado (REFUSE L62).* Nenhuma chamada a `stats()` ou `health()` pode bloquear a thread de gravação (`commit_async_ops`). Apenas locks de leitura compartilhados (`parking_lot::RwLock::read`) e operações atômicas são permitidos.

---

## 5. Status de Verificação

- **Testes Unitários de Diagnóstico:** `health_kernel::tests` (6/6 aprovados).
- **Teste de Integração de Disco Real:** `test_live_db_health_and_stats` aprovado com validação de pins, lag de sequência e saídas Prometheus/JSON.
- **Teste de Rede HTTP:** `pedradb-http::tests::kv_http_metrics_and_health_endpoints` aprovado com validação dos códigos HTTP 200/503 e payload JSON/Prometheus.
