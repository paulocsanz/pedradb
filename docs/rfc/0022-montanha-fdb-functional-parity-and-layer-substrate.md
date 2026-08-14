# RFC-0022: Paridade funcional com FDB — Montanha como primitiva de N writers

**Status:** in-progress (P0–P2 **thin proofs shipped** in-tree; not FDB field peer / not wire-compat)  
**Updated:** 2026-08-13  
**Parents:** [RFC-0013](0013-montanhadb-product.md) · [RFC-0017](0017-montanha-fdb-class-substrate.md) · [RFC-0010](0010-dbs-on-top.md) · [RFC-0019](0019-local-primitive-for-platform-and-scylla-need.md)  
**Related:** [RFC-0021](0021-montanha-fdb-tikv-parity-gaps.md) (gates de lab — **não** é o foco) · [**RFC-0023**](0023-fdb-functional-tx-parity-and-compat-face.md) (**TX physics real + fdb-compat**) · [`../montanha-vs-foundationdb.md`](../montanha-vs-foundationdb.md) · [`../node-primitive-and-unified-platform.md`](../node-primitive-and-unified-platform.md) · [`../grail-plan-build-databases-on-pedradb.md`](../grail-plan-build-databases-on-pedradb.md) · [`../foundationdb-layers-and-products.md`](../foundationdb-layers-and-products.md) · [`../fdb-limitations-analysis.md`](../fdb-limitations-analysis.md)

---

## 0. Uma frase

**Montanha deve ter paridade de *função* com FoundationDB como substrate (KV ordenado + TX multi-key + sharding escondido + layers).**  
**A prova não é battle-test de 8h — é construir em cima sistemas que *precisam* dessa física, todos horizontalmente escaláveis com N writers onde o produto exige.**

Se queres testar Montanha: faz **TiKV**, **Scylla-need**, **ClickHouse horizontal**, **SQLite em Pedra**, **Postgres N writers**, **TiDB**, **etcd** — em cima. Não mais smoke de Raft.

---

## 1. O que este RFC é / não é

| Este RFC | Não é este RFC |
|----------|----------------|
| **Paridade funcional** com FDB: ordered keys, multi-key ACID, strict-serializable-enough, layers, scale-out com clientes cegos a leaders | Paridade de **ops**, Simulation Apple-scale, Jepsen theater, “field peer” de marketing |
| **Potencial:** Montanha como **primitiva** sob produtos complexos | Wire-compat day-one com PG/MySQL/CQL/CH/etcd gRPC |
| **Prova por hosting:** layers com **N writers** concorrentes em ranges | Claim de já substituir cada produto nomeado |
| Escada de funções L0→L7 que desbloqueia cada face | Mega-slice “implementar o mundo” num PR |

**Objetivo adversarial (substrate, não cosplay):**  
Onde FDB é fraco (embed local, limites 5s/10MB/100KB no path distribuído, sem primitiva “um processo = um DB”), Pedra+Montanha **podem superar** FDB como *plataforma de layers*.  
Onde FDB é forte (ACID global, layers em produção, maturidade), **igualamos a função** antes de qualquer claim de campo.

```text
  etcd (N clients)     TiKV-like KV (N range writers)
  Postgres N-writer    TiDB-class SQL (N writers + 2PC)
  SQLite-on-Pedra      Scylla-need CP (N hot-range writers)
  ClickHouse-need      streams / NATS-need (N ingest)
           │  todos falam: chaves ordenadas + TX + (opcional) watch/log
           ▼
        Montanha Store   ← paridade funcional FDB (este RFC)
           │  N range leaders = N domínios de escrita concorrente
           ▼
        PedraDB por nó   ← kernel local (embed + apply)
```

---

## 2. Background

### 2.1 O que FDB *vende* (função, não folclore de ops)

FoundationDB, como produto de substrate, entrega:

1. **Um keyspace ordenado** (sort global).  
2. **TX multi-key ACID** como **único** caminho de mutação sério.  
3. **Serializabilidade estrita** (com limites documentados de tamanho/tempo).  
4. **Layers** (Record, Document, apps) — **sem** segundo produto de coordenação.  
5. **Scale horizontal** por sharding interno; **cliente não gerencia leader de shard**.

Máquina de commit unbundled (proxies, resolvers, TLogs) é **implementação**.  
**Paridade funcional** = (1)–(5) bem o bastante para layers sérias viverem em Montanha como vivem em FDB.

### 2.2 O que já temos (honesto — atualizado com P0–P2 thin proofs)

| Função FDB | Montanha / Pedra **agora** | Residual (não field peer) |
|------------|---------------------------|---------------------------|
| Ordered keys | Sim (Pedra + ranges) | — |
| Multi-key TX single-range | `put_batch` / `PendingTx` | — |
| Multi-key TX + OCC/snapshot | **`SnapshotTx`** + `commit_snapshot_tx` (OCC read/write set, `TransactionTooOld`) | Não é MVCC FDB-completo / SSI formal |
| Cross-range TX | `commit_tx` 2PC + SnapshotTx commit | Limites de tamanho; sem parallel-commit field |
| Cliente esconde leadership | In-process `put`/`SnapshotTx` sem node_id; TCP `TcpClusterClient` NotLeader retry | Proxy role-aware em prod multi-host ainda lab |
| Watch / change feed | **`WatchHub`** no cluster; notify pós majority put/commit_tx/DCS | Não é etcd watch gRPC; in-process hub |
| N writers | Multi-Raft ranges + **PK-leading table encode** + split/merge | Auto-rebalance PD-class incompleto |
| Layers no cluster | **EtcdNeedFace** multiproc freeze; TiKV/PG/OLAP/stream thin faces | Wire-compat day-one out of scope |
| Embed path (SQLite-class) | Pedra local + `table_*` key encode | VFS sqlite3 real não shipped |

RFC-0021 fechou **gates de lab** (perf JSON, sim scripts, rewire, region tags).  
RFC-0017 fechou **substrato multiproc TCP**.  
RFC-0022 thin proofs: **funcionalidade de layer com N writers** — ainda **não** “hospedamos Postgres/TiDB de produção”.

### 2.3 Por que “teste de batalha” é o critério errado *aqui*

| Critério | O que prova | O que **não** prova |
|----------|-------------|---------------------|
| Soak 8h / fault wall | Robustez sob stress | Que layers complexas *cabem* na API |
| Jepsen-class partitions | Correctness de consenso | Que SQL/índice secundário/DCS usam a mesma física |
| **Layer com N writers** | Substrate tem a física FDB | Pedigree de 10 anos de cloud |

0020/0021/0018 cobrem método e stress.  
**0022 cobre capacidade de produto:** *potencial* de ser a primitiva sob sistemas maiores.

---

## 3. Problems this solves

- **Problem:** “Paridade com FDB” virou discussão de battle-test; builders precisam de **mapa de capacidade**.  
- **Problem:** Layers (SQL, DCS, OLAP, streams) dual-escrevem em etcd/Scylla/CH se o substrate não carrega N writers + TX.  
- **Problem:** Critério de sucesso de Montanha era auto-referente (mais smoke de Raft) em vez de **hospedar sistemas que precisam de física FDB**.  
- **Problem:** Lista de ambição (TiKV, Scylla, CH, SQLite, PG, TiDB, etcd) sem dependência ordenada nas funções do substrate.  
- **Problem:** Confundir “FDB field peer” com “FDB-shaped substrate bom o bastante para layers.”

---

## 4. Proposed solution

### 4.1 Tese (normativa)

1. **Montanha faz o trabalho do FDB:** store transacional ordenado multi-writer em que layers compõem.  
2. **Pedra faz o trabalho do kernel por nó** (e o que FDB nunca ofereceu bem: **embed**).  
3. **Validação de Montanha = shipping de layers com N writers**, não só caos interno.  
4. **Se um produto ainda precisa de etcd/Scylla/CH/NATS para *corretude***, Montanha ainda não absorveu essa need.  
5. **Wire-compat é opcional e tardio;** semântica + N writers vêm primeiro.

### 4.2 Escada de funções (subir em ordem)

| L | Função (FDB-like) | Desbloqueia… |
|---|-------------------|--------------|
| **L0** | Put/get ordenado durável + majority | KV cru, demos embed |
| **L1** | TX multi-key atómico + API sem range leaders no app | App TX, SQL single-shard |
| **L2** | Snapshot/OCC (ou SSI equivalente) + classes de conflito | Apps concorrentes, port de layers FDB |
| **L3** | TX cross-range com limites claros | Índice secundário global, SQL multi-tabela |
| **L4** | Watch / change log cluster-correct | etcd-like DCS, invalidação, CDC |
| **L5** | Placement: split/merge + N range leaders estáveis | **N writers** em escala (TiKV / Scylla-need QPS) |
| **L6** | RO/learner ou projeção | ClickHouse-need scans sem dual SoR |
| **L7** | Tenant/namespace + auth hooks | Plataforma multi-produto |

**Hoje:** L0–L1 parciais, L3 parcial, L4–L7 finos.  
**P0–P2 deste RFC:** subir **L1→L5** de verdade; L6–L7 e faces completas na sequência.

### 4.3 Como testar Montanha: faces de produto (todas com N writers onde couber)

Cada linha é uma **layer** (ou protocol face), **não** reimplementação wire-compat do competidor:

| Face-alvo | O que pede a Montanha | Modelo de writers | Competidor conceptual |
|-----------|----------------------|-------------------|------------------------|
| **etcd** | L1–L4: TX + CAS/create + watch | Muitos clients; leaders por meta-range | etcd / Consul / ZK |
| **TiKV-like KV** | L1–L3 + L5: KV + TX opcional + split | **N range leaders** | TiKV / FDB raw |
| **Scylla-need CP** | L1 + L4 + L5: QPS alto, watch/push | N writers em ranges quentes | *need* de Scylla no CP (não AP multi-master) |
| **SQLite em Pedra** | Pedra local ± sync via Montanha | 1 writer local; multi-nó via store | SQLite / LiteFS-class |
| **Postgres N-writer** | L2–L5 + SQL layer: PK→range | **N writers** por range de PK | PG + Citus / multi-primary sharding |
| **TiDB-class SQL** | L2–L5 + planner distribuído | N writers + 2PC/parallel-commit class | TiDB / CRDB |
| **ClickHouse horizontal** | L6: RO projection / learner do log SoR | Writers no path OLTP; **N scanners RO** + N ingest | ClickHouse-need |
| **Stream / NATS-need** | Log-derived subjects | N publishers; consumers com cursor | JetStream-need |

**Regra de ouro:** layer só é válida se **não** exigir segundo store durável de coordenação para estar correta.

### 4.4 Onde podemos *superar* FDB como substrate (potencial)

Não é marketing de “melhor que Apple.” É **função + potencial de stack**:

| Dimensão | FDB | Pedra + Montanha (alvo) |
|----------|-----|-------------------------|
| Embed / library | Cluster-only | **Pedra** = SQLite-class local + mesma física de keys/TX |
| Limites de TX | 5s / 10MB / 100KB (path distribuído) | Path local sem esses tetos; path cluster com limites **honestos e documentados** |
| N writers | Sharding interno; cliente cego | **Range leaders explícitos** como ferramenta de escala (identidade de produto continua FDB-shaped) |
| OLAP | Layers/apps; não warehouse nativo | **L6** RO projection no mesmo SoR (HTAP path) |
| DCS | Layer (e.g. etcd-on-FDB experiments) | **Primeira-class layer** no mesmo store (sem etcd process) |
| Um kernel → muitos produtos | Sim (layers) | Sim + **embed** no mesmo repo de primitiva |

**Superar FDB** neste RFC = ser **melhor plataforma de layers** (embed + N writers + HTAP RO + DCS na mesma física), **não** “Simulation day-one.”

### 4.5 Arquitetura fixa

```text
┌──────────────────────────────────────────────────────────────┐
│  Product faces (crates / serviços futuros)                   │
│  etcd-need · tikv-kv · pg-nwriter · tidb-sql · olap-ro       │
│  scylla-need-cp · sqlite-on-pedra · stream-nats-need         │
└────────────────────────────┬─────────────────────────────────┘
                             │ begin / get / set / commit
                             │ watch · range scan · place
┌────────────────────────────▼─────────────────────────────────┐
│  Montanha Store — ranges · majority · placement · TX         │
│  N leaders ⇒ N domínios de escrita concorrente               │
└────────────────────────────┬─────────────────────────────────┘
                             │ apply_batch / commit
┌────────────────────────────▼─────────────────────────────────┐
│  PedraDB (por nó) — ordered keys · multi-key TX local        │
└──────────────────────────────────────────────────────────────┘
```

### 4.6 Dependência entre faces (ordem de prova)

```text
L0–L1  ──►  SQLite-on-Pedra (embed)
         └─►  TiKV-like thin KV
L2–L3  ──►  secondary index TX  ──►  Postgres N-writer  ──►  TiDB spine
L4     ──►  etcd-need  ──►  Scylla-need CP (com L5)
L5     ──►  N writers reais em todas as faces acima
L6     ──►  ClickHouse-need RO (+ N ingest writers no SoR)
L4+log ──►  stream / NATS-need
```

Não pular L2 para “SQL distribuído completo.” Não pular L4 para “somos etcd.”

---

## 5. Delivery slices

### P0 — funções que layers realmente chamam (sem foco em battle-sim)

- [x] **P0.1** **TX serializable-enough (single-range first)** — status: `done`  
  - `SnapshotTx` + `snapshot_begin` / `commit_snapshot_tx`: snapshot generation, OCC on read/write set, `TransactionTooOld` via `MAX_SNAPSHOT_LAG`.  
  - Evidence: `layers::tests::snapshot_tx_*` (begin/commit, OCC WW, read-set, too-old).

- [x] **P0.2** **Write path leadership-invisible como default de produto** — status: `done`  
  - Layers use `put` / `SnapshotTx` / `put_routed_invisible` without node ids; TCP `TcpClusterClient` NotLeader retry (RFC-0021).  
  - Evidence: all face tests + `n_writers_disjoint_ranges`.

- [x] **P0.3** **Watch / notify em prefixo de chave (cluster-correct)** — status: `done`  
  - `WatchHub` on `StoreCluster`; notify after majority put / put_batch / commit_tx / DCS.  
  - Evidence: `watch_after_majority_put`, etcd-need watch.

- [x] **P0.4** **Layer freeze #2: etcd-need face só em multi-process store** — status: `done`  
  - `EtcdNeedFace` create/CAS/get/watch via store only; multiproc smoke `etcd-need` / `etcd-need-verify`.  
  - Evidence: `multi_process_etcd_need_face_freeze`.

### P1 — N writers + índices + faces SQL/KV

- [x] **P1.1** **TX cross-range: limites de produto + padrão de secondary index** — status: `done`  
  - `put_with_secondary_index` via `SnapshotTx` (data + index keys atomic).  
  - Evidence: `secondary_index_via_tx` (+ existing `commit_tx` cross-range).

- [x] **P1.2** **Placement para N writers (split/merge + hook de balance)** — status: `done`  
  - `split_range_at` + `merge_adjacent_ranges`; multi-range open + N writers test.  
  - Evidence: `n_writers_disjoint_ranges`.

- [x] **P1.3** **TiKV-like raw KV face (layer fina)** — status: `done`  
  - `TikvKvFace` put/get/delete/batch_put.  
  - Evidence: `tikv_face_n_writers_and_batch`.

- [x] **P1.4** **SQLite-on-Pedra (embed) + sketch de sync Montanha** — status: `done`  
  - Table key encoding **PK-leading** `{pk}\0t/{table}/r` (+ index `{val}\0i/...`) so ranges can split on PK.  
  - Evidence: `table_sqlite_encode_and_pg_nwriter` (encode roundtrip).

- [x] **P1.5** **Postgres N-writer slice (mínimo)** — status: `done`  
  - `pg_upsert` / `pks_one_per_range`; **assert `locate(pk_i) ≠ locate(pk_j)`** + same-PK OCC.  
  - Evidence: `table_sqlite_encode_and_pg_nwriter`.

### P2 — faces de plataforma (OLAP, SQL dist, Scylla-need, streams)

- [x] **P2.1** **ClickHouse-need RO path (horizontal)** — status: `done` (thin)  
  - `olap_ingest` / `olap_get` under `olap/{stream}/{seq}` same SoR.  
  - Evidence: `olap_ro_and_stream_from_sor`.

- [x] **P2.2** **TiDB-class distributed SQL spine** — status: `done` (thin)  
  - `sql_multi_table_write` multi-table rows in one `SnapshotTx`.  
  - Evidence: `olap_ro_and_stream_from_sor`.

- [x] **P2.3** **Scylla-need high-QPS CP profile** — status: `done` (thin)  
  - CP keys under `cp/` + watch after majority put.  
  - Evidence: `scylla_need_cp_put_watch`.

- [x] **P2.4** **Stream / NATS-need durable subjects** — status: `done` (thin)  
  - `stream_publish` / `stream_get` under `stream/{subject}/{seq}`.  
  - Evidence: `olap_ro_and_stream_from_sor`.

---

## 6. Status (living — atualizar com cada PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Serializable-enough single-range TX API | done | `SnapshotTx` + `layers::tests::snapshot_tx_*` | 2026-08-13 |
| P0.2 | p0 | Leadership-invisible writes default | done | `put`/`SnapshotTx`/`TcpClusterClient`; faces | 2026-08-13 |
| P0.3 | p0 | Cluster-correct key prefix watch | done | `WatchHub` + `watch_after_majority_put` | 2026-08-13 |
| P0.4 | p0 | etcd-need layer freeze multi-process | done | `EtcdNeedFace` + `multi_process_etcd_need_face_freeze` | 2026-08-13 |
| P1.1 | p1 | Cross-range TX + secondary index pattern | done | `put_with_secondary_index` + test | 2026-08-13 |
| P1.2 | p1 | Placement for N writers | done | `split_range_at` + `merge_adjacent_ranges` + n_writers | 2026-08-13 |
| P1.3 | p1 | TiKV-like raw KV face | done | `TikvKvFace` + `raw_keys_one_per_range` (assert ≥3 ranges) | 2026-08-13 |
| P1.4 | p1 | SQLite-on-Pedra (+ Montanha sync sketch) | done | PK-leading `table_row_key` + encode roundtrip | 2026-08-13 |
| P1.5 | p1 | Postgres N-writer minimal slice | done | `pks_one_per_range` + `locate` multi-range assert | 2026-08-13 |
| P2.1 | p2 | ClickHouse-need RO path (horizontal) | done | `olap_*` thin SoR path | 2026-08-13 |
| P2.2 | p2 | TiDB-class SQL spine | done | `sql_multi_table_write` thin | 2026-08-13 |
| P2.3 | p2 | Scylla-need CP profile | done | `cp/` + watch thin | 2026-08-13 |
| P2.4 | p2 | Stream / NATS-need | done | `stream_*` thin | 2026-08-13 |

**Honesty residual:** faces are **thin encodings + tests**, not production TiDB/PG/CH/Scylla/etcd wire. Surpass-FDB *as substrate* = layer proofs on N-writer physics; **not** ops/Simulation supersession.

---

## 7. Capability matrix (alvo vs agora)

| Ambição de produto | Ladder mínimo | Agora (lab thin) | Bloqueador residual |
|--------------------|---------------|------------------|---------------------|
| **etcd** (HA locks/config, N clients) | L1–L4 | EtcdNeedFace + multiproc freeze | etcd gRPC wire |
| **TiKV-like KV** N writers | L1 + L5 | TikvKvFace + multi-range | PD auto-rebalance field |
| **Scylla-need CP** N writers | L1 + L4 + L5 | `cp/` + watch | load/QPS profile field |
| **SQLite embed** | Pedra L0 local | table key encode | real VFS/sqlite3 link |
| **Postgres N writers** | L2–L5 + SQL | pg_upsert + OCC | PG wire / catalog |
| **TiDB-class** N writers | L2–L5 + planner | multi-table SnapshotTx | planner/optimizer |
| **ClickHouse horizontal** | L6 (+ ingest N) | olap seq keys RO | column projection/learner |
| **Stream NATS-need** | L4 + log | stream seq keys | NATS protocol / fanout |
| **Tudo numa plataforma** | L0–L7 | thin faces green | product packaging |

---

## 8. Acceptance criteria

### Functional (primário)

- **P0:** Clientes TX concorrentes + DCS via watch correm **só** em Montanha multiproc/TCP; **sem** etcd externo.  
- **P1:** ≥2 writers concorrentes em ranges disjuntos; secondary index via TX; demo PG-shaped N-writer; face TiKV-like com N writers; path SQLite-on-Pedra.  
- **P2:** Pelo menos um proof OLAP RO a partir do SoR; subjects de stream sem NATS para corretude; spine SQL multi-range com testes.

### Tests (outcomes nomeados — não horas de soak)

- `tx_snapshot_or_occ_conflict_*`  
- `watch_after_majority_commit_*`  
- `dcs_layer_no_external_etcd_*`  
- `n_writers_disjoint_ranges_*`  
- `secondary_index_tx_*`  
- `sqlite_or_table_encode_roundtrip_*`  
- `pg_nwriter_disjoint_pk_*`  
- `tikv_face_n_writers_*`  
- `olap_ro_from_sor_*` (P2)  
- Layer freezes: multiproc only (mesma doutrina RFC-0017 P2.3).

### Telemetry

- Per-range leader, commit/apply index (já existe).  
- Contadores de conflito de TX; watch lag; writers-active por range.

### Documentation

- Tabela Status deste RFC.  
- [`node-primitive-and-unified-platform.md`](../node-primitive-and-unified-platform.md) aponta aqui para “como provamos Montanha.”  
- RFC-0021 = residual de lab/gates; **0022 = escada funcional de produto.**  
- Não claimar FDB field peer no README até matrix §7 estar majoritariamente `done` **e** faces com N writers green.

### Screenshots

- backend-only (diagrama de arquitetura opcional).

---

## 9. Out of scope

- Wire-compatible Postgres / MySQL / CQL / ClickHouse / etcd gRPC **day one**.  
- Substituir Pedra por Rocks sob Montanha.  
- Dual SoR durável (app escreve Montanha **e** etcd/Scylla/CH).  
- Claim de paridade de **campo** com FDB ou Simulation Apple.  
- Usar paredes de sim 8h/24h como **definição** de sucesso deste RFC (0021/0018 donos desses gates).  
- Scylla **AP multi-master** (física diferente); só **Scylla-need CP** no path ACID.

---

## 10. Relationship to other RFCs

| RFC | Papel |
|-----|--------|
| **0010** | Layers in-process charter (feito); 0022 eleva para **cluster + N writers** |
| **0017** | Substrate multiproc/TCP existe |
| **0018** | Método FDB (fault coverage) — complementar, não substitui 0022 |
| **0019** | Pedra local L1 / seeds Scylla-need |
| **0020** | Synthetic field maturity (kernel) |
| **0021** | Gates de lab (perf/sim/ops) |
| **0022 (este)** | **Paridade funcional FDB + primitiva para produtos N-writer** |

---

## 11. Normative sentences

1. **Montanha valida-se por layers com N writers**, não só por self-tests de Montanha.  
2. **Paridade funcional com FDB** = ordered keys + multi-key TX + sharding escondido do app + layers — **não** o grafo unbundled de processos.  
3. **Se o produto ainda precisa de etcd / Scylla / CH / NATS para corretude**, a need ainda não foi absorvida.  
4. **SQLite / Postgres / TiDB / TiKV / etcd / OLAP faces são layers**; Pedra e Montanha ficam a física.  
5. **Testar Montanha = construir em cima** (TiKV, Scylla-need, CH horizontal, SQLite-on-Pedra, PG N-writer, TiDB, etcd) — todos horizontalmente escaláveis com N writers onde o modelo exige.  
6. **Superar FDB como substrate** (embed + HTAP RO + DCS na mesma física + limites honestos no path local) é o norte de **potencial**; field pedigree é outro RFC e outra década.
