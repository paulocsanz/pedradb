# Catálogo: dependentes de RocksDB + os benchmarks deles (para substituir e checar)

**Updated:** 2026-08-19
**Dono:** [RFC-0043](rfc/0043-high-level-2x-expanding-benches.md) ("catálogo que só cresce").
**Pergunta que este doc responde:** quais DBs usam RocksDB internamente, que benchmark
público existe para cada um, e como a Pedra entra — troca in-process, shape no runner,
ou bloqueado.

**Regra de ouro (AGENTS.md):** o cartaz “batemos o Rocks” é sempre vs
**RocksDB default `WriteOptions.sync=false`**. Nas suítes de dependente
(`kvrocks` / `myrocks` / `surreal`) o peer usa o **default do DB de cima**:
Kvrocks async, MyRocks sync-on-commit, Surreal `sync=every`. Isso **não**
é um win oficial vs Rocks default. JSON `peer_policy=host-default`.

**Superfície do `rocksdb-compat` hoje** (leitura de `crates/rocksdb-compat/src/lib.rs`):
`Options`, `ColumnFamily` (emulação por prefixo `cf\0key`), `DB::open/open_cf`,
`put/put_cf/get/get_cf/delete/delete_cf/delete_range_cf`, `WriteBatch` multi-CF
(`write`, `write_owned`, `write_cf_slices`, `write_cf_owned`), `Snapshot`
(get/iterator), `DBIterator` (`valid/next/key/value/collect_rest`, `IteratorMode`),
`last_key_with_prefix`, `count_cf/count_named`, `flush`, `compact`.
**Não tem:** ingest externo/SSTFileWriter, compaction filter, properties/estatísticas,
Titan/BlobDB, PlainTable, merge operator, `multi_get`, checkpoint, transações
(pessimistas **ou** otimistas).

---

## Classe A — troca in-process plausível (Rust + binding rocksdb-typed)

Onde "substituir e checar" é literal: build do DB com o binding apontando pro nosso
`rocksdb-compat`, bench deles rodando igual nos dois mundos.

### A1. SurrealDB (`kv-rocksdb`) — alvo nº 1

- **Papel do RocksDB:** engine de armazenamento local recomendada pra produção
  single-node (`surreal start rocksdb://…`). SurrealKV existe (beta) mas **não**
  substituiu RocksDB — docs dizem "prefer RocksDB" pra deploy on-disk conservador.
- **Binding:** `surrealdb-rocksdb` (fork do rust-rocksdb) em
  `surrealdb/core/src/kvs/rocksdb/mod.rs`. Usa **`OptimisticTransactionDB`**:
  `transaction_opt` com `set_snapshot(true)`, CF `default` (descriptor explícito só
  quando versioning), commits via batch, cursor/iterate com snapshot.
  **`WriteOptions::set_sync(false)` por padrão** — comentário no código: per-tx sync
  nunca é usado; sync só no coordinator quando `sync=every`. Ou seja: o padrão deles
  **é exatamente o nosso peer oficial** (`sync=false`).
- **Benchmarks públicos (deles):** `crud-bench` (Rust, open source, roda em CI diário
  no GitHub Actions) — CRUD/scan/filter/index; fork do `go-ycsb` (de PingCAP) no repo
  `surrealdb/benchmarking`; página `surrealdb.com/benchmarks` com números 3.x.
  Baseline publicada: Hetzner CCX63 (48 vCPU, 192 GB, NVMe), 5M records.
- **Gap no compat:** OCC + compile-shape rust-rocksdb **shipped** (RFC-0043
  P2.4): `transaction_opt` + snapshot + get/put/delete/`raw_iterator_opt` +
  `Busy`, `open_cf_descriptors`, `ReadOptions`, `property_int_value`,
  `flush_opt`/`flush_wal`/`compact_range_opt`. Options tunables e prefix
  extractor são no-op. Ainda falta para build in-process: UDT timestamps
  (comparador de verdade), bloom de prefixo, o resto do `surrealdb-rocksdb`
  0.24 (MemoryManager / write-buffer-manager FFI).
- **Suíte no runner:** `ROCKS_PARITY_SUITE=surreal` —
  `surreal_tx_get` (HL), `surreal_tx_put` (canário), `surreal_tx_rmw` (HL),
  `surreal_tx_scan` (HL), `surreal_tx_batch` (HL). Peer Rocks =
  `OptimisticTransactionDB` `sync=false`. Remesura 3× pendente.
- **Por que é primeiro:** bench público + diário, workload multi-model CRUD
  (put/get/scan/filter), default sync=false já alinhado com o peer oficial.

### A2. Oxigraph — confirmado (2026-08-19)

- Ainda RocksDB: crate 0.5.9 (Jun 2026), feature default `rocksdb`,
  `oxrocksdb-sys` bump Jun 2026. Store “repeatable read”. WASM desliga
  Rocks e cai no in-memory.
- **Não** é troca in-process hoje (binding próprio `oxrocksdb-sys`, não
  rust-rocksdb). Shape no runner: suíte opt-in `oxigraph` —
  `oxigraph_spo_lookup` (point get SPO) + `oxigraph_triple_put` (batch
  insert). **Não** é SPARQL / algebra.

### A3. TiKV — parcial (já é o nosso modelo, não uma troca)

- Usa `rust-rocksdb` (fork tikv), multi-CF (`default`/`write`/`lock`/`raft`),
  compaction filter pra MVCC GC, Titan em produção grande, ingest. **Não** é troca
  in-process viável hoje (filtros/ingest/Titan = Classe C). O que fazemos dele são
  shapes — já modeladas na suíte `deps` (apply/prewrite/mvcc/scan/raftlog) com
  proveniência TiKV. Benchmarks oficiais deles: **go-ycsb** (tikv.org docs v5.1/v6.1:
  3 nós, 10M records, ~212k point-get/s C, ~43k update/s A, <10 ms) e, no nível TiDB,
  sysbench + go-tpc (guides oficiais PingCAP). Esses números definem o mix que as
  shapes `deps_*`/`ycsb_*` já reproduzem no nosso harness.

## Classe B — extrair shapes (bench roda contra o DB inteiro; Rocks não é swappable, o workload é)

Mesmo `Engine` trait, mesmo seed, dois adapters (compat + rocks default), nome estável
no `summarize`, entrada nova no compare — append, nunca delete ([RFC-0043 §Como adicionar](rfc/0043-high-level-2x-expanding-benches.md)).

| DB | Papel do RocksDB | Benchmark público | Dado publicado de referência | Shapes propostas | Notas de API p/ shape |
|---|---|---|---|---|---|
| **Kvrocks** (Apache) | storage inteiro; Redis-protocol em cima; WriteBatch no write path | `redis-benchmark`, `memtier_benchmark` (compatível por protocolo) | Discussão #389: GET ~96k rps / SET ~58k rps (4 vCPU NVMe); blog oficial: hash index −21.8% CPU/+10% tput; BlobDB 2.5–5× write p/ valores 10–50 KB | **no runner** (`ROCKS_PARITY_SUITE=kvrocks`): `kvrocks_get` (canário), `kvrocks_set` (canário), `kvrocks_pipelined_set` (HL), `kvrocks_scan` (HL) | Sem bench oficial publicável; shapes derivam do protocolo + posts. Payload = `ROCKS_YCSB_PAYLOAD`. Remesura 3× pendente |
| **MyRocks** (MySQL/MariaDB/Percona) | storage engine do MySQL | sysbench OLTP (`oltp_point_select`, `oltp_read_only`, `oltp_write_only`…), `sysbench-tpcc` (Percona-Lab), **LinkBench** (Meta, social graph) | Percona 2018 tpcc: MyRocks ~4.2–4.6k TPS flat vs InnoDB 0.85–6k (memória-dependente); 2022 IO-bound: >575k vs 125k tx/h; Small Datum 2023: 42 microbenches point/range/write; LinkBench: MyRocks 2× menor que InnoDB comprimido, QPS competitivo | **no runner** (`ROCKS_PARITY_SUITE=myrocks`): `myrocks_point_select` (canário), `myrocks_read_only` (HL), `myrocks_write_tx` (HL), `linkbench_mix` (HL; 55% GET_LINKS_LIST / 15% GET_NODE / 25% ADD-UPDATE batch / 5% DELETE) | READ-COMMITTED (sem gap locks) — batch por tx é o modelo natural. Remesura 3× pendente |
| **NebulaGraph** | KVStore em cima de RocksDB (vertices/edges codificados, Raft por partição, ~24+ parts) | **nebula-bench** (LDBC SNB SF100: ~282M vertices/1.77B edges; k6 + xk6-nebula; GO 1–3 hops, MATCH, LOOKUP, INSERT) | Relatórios oficiais 3.4/3.5.0 por versão (não auditados LDBC); SF1000 (~500 GB) no 1.0 | **no runner** (`ROCKS_PARITY_SUITE=nebula`): `nebula_get_neighbors`, `nebula_insert_edge` | getNeighbors = scan curto; multi-disk fora do escopo |
| **ArangoDB** | engine único desde 3.7 (RocksDB) | `arangobench` (document CRUD/insert, concurrency, sync), benchmark 2018 NoSQL (Pokec 1.6M v/30.6M e; shortest-path ~416 ms/1000 paths) | Posts oficiais + blog 2018 com tabela vs Mongo/Postgres/Orient/Neo4j | **no runner** (`arango`): `arango_doc_crud`, `arango_traversal` | Locking document-level ≠ CF; shape é workload, não arquitetura |
| **Ceph (BlueStore)** | kvstore de **toda metadata** (omap, extents, PG logs); dado do usuário vai direto pro bloco | `rados bench`, `fio` (RBD/CephFS), `ceph-bench` (rand write por OSD, size=1) | Mark Nelson 2022: ~527–577k 4K-rand-write IOPS (10 nós NVMe, tuning de memtable); 2024: rebuild RelWithDebInfo ~2× IOPS | **no runner** (`ceph`): `bluestore_omap_write`, `bluestore_omap_read`. Peer = host-default **sync** (omap durável) | O gargalo publicado é o kv de metadata, não o write do dado |
| **Flink / Kafka Streams** | state backend (RocksDB state store / EmbeddedRocksDBStateBackend) | **Nexmark** (suite Flink oficial; q0–q20, 100M eventos), Yahoo Streaming Benchmark, Theodolite (JSS 2024) | Nexmark repo: q0 ~155k ev/s/core; q5/q7/q9/q16/q20 stateful custam ordens de magnitude mais; incremental checkpoints recomendados | **no runner** (`streaming`): `flink_window_state`, `kafka_changelog_flush` | JVM fora (shape só) |
| **Solana (Agave)** | Blockstore/ledger inteiro (shreds, status, metadata) via crate `rocksdb`; `agave-ledger-tool` para inspecionar/compactar | sem bench oficial público; issues documentam stalls ~40 min, write spikes; **Sig** (Syndica, Zig) tem ledger pluggável (LMDB/mem) e post de engenharia jan/2025 | Issues #16234 etc.; TerarkDB (fork ByteDance) citado como drop-in potencial | **no runner** (`solana`): `solana_shred_append`, `solana_trailing_read` | TerarkDB é fork C++; Sig confirma ledger pluggável |
| **Venice (LinkedIn)** | engine de storage (desde ~2018; PlainTable p/ serving em memória, BlobDB p/ valores grandes, SSTFileWriter no ingest) | sem suite pública; QCon 2024: 1M ops/s/nó (32c/256GB; ~620k gets + ~680k writes); blog: 175M lookups/s + 230M writes/s cluster, SLA <10 min write | QCon/InfoQ "scalable-low-latency"; blogs LinkedIn (fanout, ingestion pipeline 2025) | **no runner** (`venice`): `venice_fanout_get` (**32** gets/op, scaled-down do 5k+ publicado). `venice_ingest_batch` continua **blocked** (SSTFileWriter) | PlainTable/BlobDB = classe C |
| **Pinterest Rockstore(+wide column)** | storage de todos os serviços Rocksplicator; chave = row_key+col+ts com comparador ts descendente | sem microbench público | Blogs Pinterest: 300+ casos, milhões rps, PBs, ms single-digit; Rocksplicator: 9 sistemas, dezenas de M qps, 50M+ inferências ML/s | **no runner** (`venice`): `rockstore_widecol_rw` | Rocksplicator é open source (C++); bench não |
| **Rockset (→OpenAI)** | índice convergido em cima de RocksDB | blogs com bench próprios; TSBS-likes | posts Rockset eng | `rockset_hybrid` (ingest batch + query point/scan misto) | baixa prioridade (fontes esparsas) |
| **ZippyDB / Quicksilver (Meta/CF)** | ZippyDB: serviço KV thrift (Titan, sampling do paper Dong 2021 = R010); Quicksilver: config fabric edge | **não públicos** | Paper Dong 2021 (amostragem de 42 apps ZippyDB/MyRocks = R010 no research/); blog CF Quicksilver | já modelado: `qs_hot_get`, `qs_neg_lookup`, `qs_batch_write` (RFC-0043 HL₁) | QS não é open source — shapes são inspiradas em números publicados, não replay |
| **Twitter Manhattan** | backend RocksDB do KV distribuído | nenhum público | — | fora da fila até existir fonte pública | — |

## Classe C — bloqueado até API existir (sem stub que o Rocks faz e a Pedra finge)

- **TiKV produção completa:** compaction filter (MVCC GC), ingest/SSTFileWriter
  (snapshot/restore), Titan (values grandes), properties p/ region size. Shape de
  workload ok; troca in-process não.
- **Venice ingest (SSTFileWriter)**: mesmo gap de ingest.
- **Kvrocks BlobDB p/ valores grandes** (2.5–5× write publicado): blob/value-log é
  API separada (cf. RFCs 0026–0029 do lado Pedra).
- **PlainTable (Venice serving em RAM)**: formato/API que não temos; o que dá pra
  testar sem ele é o hot-set-cacheado (já coberto por YCSB-C-like).

## Excluídos do catálogo com motivo (não reabrir sem fonte nova)

- **Qdrant**: usava RocksDB p/ payload/sparse; **substituiu por Gridstore** no 1.13
  (blog "gridstore-key-value-storage": latência mais estável, sem compaction online;
  IDs sequenciais). Não é mais dependente.
- **Milvus**: RocksDB só no **RocksMQ** (WAL/message queue standalone; cluster usa
  Kafka/Pulsar e novos defaults movem pra Woodpecker); vetores/índices vão pra object
  storage, metadata pra etcd. Não é substituição de storage principal.
- **CockroachDB** = Pebble (próprio); **etcd** = bbolt; **Scylla** = próprio;
  **Dgraph** = Badger; **InfluxDB** = TSM/Parquet; **SurrealKV** = engine própria
  (compete com RocksDB no SurrealDB, não dependente).
- **TerarkDB (ByteDance)**: fork do RocksDB, não dependente do upstream — bench de
  fork vs fork não é o nosso peer.

## Fila priorizada (proposta — vira P2.3 no RFC-0043)

1. **P0 — Kvrocks shapes**: **no runner** (+ `kvrocks_set_mc50`,
   `kvrocks_blob_set` 16 KiB). Remesura 3× pendente.
2. **P0 — MyRocks shapes**: **no runner**. Remesura 3× pendente.
3. **P1 — SurrealDB**: OCC no compat; suíte `surreal` + `surreal_tx_rmw_mc8`.
   Troca in-process ainda bloqueada (UDT / prefix bloom / FFI).
4. **P1 — NebulaGraph**: **no runner** (`nebula`).
5. **P2 — ArangoDB, Ceph, Flink/Kafka, Solana, Venice/Rockstore, Oxigraph**:
   **no runner** (suítes opt-in). Ingest SSTFileWriter / BlobDB-API / PlainTable
   continuam classe C.
6. **P2 — Rockset**: só com fonte de bench executável.

## Fontes (lidas 2026-08-18)

- SurrealDB deploy/docs + deep-dive: `surrealdb.com/docs/build/deployment`,
  `surrealdb.com/deep-dive`; benchmarks: `surrealdb.com/blog/beginning-our-benchmarking-journey`
  (11/02/2025), `surrealdb.com/blog/surrealdb-3-0-benchmarks-a-new-foundation-for-performance`,
  `surrealdb.com/benchmarks`; código: `surrealdb/surrealdb` `surrealdb/core/src/kvs/rocksdb/mod.rs`
  (via raw.githubusercontent) e `surrealdb/rust-rocksdb` `src/transactions/optimistic_transaction_db.rs`;
  discussão YCSB/TiKV: `github.com/orgs/surrealdb/discussions/3413`; crud-bench:
  `github.com/surrealdb/crud-bench`.
- Qdrant/Gridstore: `qdrant.tech/articles/gridstore-key-value-storage/`,
  `qdrant.tech/blog/qdrant-1.13.x/`, `qdrant.tech/documentation/manage-data/storage/`.
- Milvus: `milvus.io/docs/mq_rocksmq.md`, `milvus.io/docs/main_components.md`,
  `milvus.io/docs/architecture_overview.md`; RocksDB USERS.md
  (`github.com/facebook/rocksdb/blob/main/USERS.md`).
- Solana/Agave: issue `solana-labs/solana#16234`; releases `anza-xyz/agave`;
  Sig ledger: `blog.syndica.io/sig-engineering-part-5-sigs-ledger-and-blockstore/`;
  TerarkDB (HN 25514419).
- Kvrocks: `kvrocks.apache.org`; discussão `apache/kvrocks#389`; blog
  `kvrocks.apache.org/blog/how-we-use-rocksdb-in-kvrocks/`.
- MyRocks: `engineering.fb.com/2016/08/31/core-infra/myrocks-a-space-and-write-optimized-mysql-database/`;
  Percona: "a-look-at-myrocks-performance", "a-myrocks-use-case",
  "benchmarking-myrocks-vs-innodb-in-memory-constrained-environments";
  Small Datum: `smalldatum.blogspot.com/2017/12/tpcc-mysql-io-bound-high-concurrency.html`,
  `smalldatum.blogspot.com/2023/04/myrocks-vs-innodb-with-sysbench.html`;
  Percona-Lab-results/201803-sysbench-tpcc-myrocks; LinkBench: `mdcallag/linkbench`;
  VLDB/MSST MyRocks (Matsunobu et al. 2017 — ref [65] do R013).
- NebulaGraph: `nebula-graph.io/posts/nebula-graph-storage-engine-overview`,
  `nebula-graph.io/posts/nebulagraph-benchmark-3.5.0`,
  `github.com/nebula-contrib/NebulaGraph-Bench`.
- Flink/Kafka Streams: `github.com/nexmark/nexmark`, `CRC-FONDA/nexmark`,
  `flink.apache.org/2021/01/18/using-rocksdb-state-backend-in-apache-flink-when-and-how/`,
  Henning & Hasselbring arXiv:2303.11088 (JSS 2024).
- Pinterest: `medium.com/pinterest-engineering/building-pinterests-new-wide-column-database-using-rocksdb-f5277ee4e3d2`,
  `…/automated-cluster-management-and-recovery-for-rocksplicator-f1f8fd35c833`,
  `…/open-sourcing-rocksplicator-a-real-time-rocksdb-data-replicator-558cd3847a9`,
  `github.com/pinterest/rocksplicator`.
- Venice: `linkedin.com/blog/engineering/open-source/supporting-large-fanout-use-cases-at-scale-in-venice`,
  `linkedin.com/blog/engineering/infrastructure/evolution-of-the-venice-ingestion-pipeline`,
  `linkedin.com/blog/engineering/code/building-venice-a-production-software-case-study`,
  InfoQ/QCon 2024 "scalable-low-latency".
- Ceph: `docs.ceph.com/en/latest/rados/configuration/bluestore-config-ref/`,
  `ceph.io/en/news/blog/2022/rocksdb-tuning-deep-dive/`,
  `ceph.io/en/news/blog/2024/silver-bullet-rocksdb-performance/`,
  `github.com/vitalif/ceph-bench`.
- ArangoDB: `arangodb.com/2018/02/nosql-performance-benchmark-2018-mongodb-postgresql-orientdb-neo4j-arangodb/`,
  docs storage-engine (3.3/3.7 transitions), Meltdown/Spectre post (medium/@neunhoef).
- TiKV/TiDB: `tikv.org/docs/5.1/deploy/performance/instructions/`,
  `tikv.org/docs/6.1/deploy/performance/overview/`,
  `docs.pingcap.com/tidb/stable/benchmark-tidb-using-sysbench/`,
  `docs.pingcap.com/tidb/stable/benchmark-tidb-using-tpcc/`,
  `github.com/pingcap/go-ycsb`, `github.com/pingcap/go-tpc`,
  `github.com/tikv/terraform-tikv-bench`.
