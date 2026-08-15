# Fichamento: R044 — FoundationDB Record Layer

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R044`

## Referência

CHRYSAFIS, Christos; COLLINS, Ben; DUGAS, Scott; DUNKELBERGER,
Jay; EHSAN, Moussa; GRAY, Scott; GRIESER, Alec; HERRNSTADT,
Ori; LEV-ARI, Kfir; LIN, Tao; MCMAHON, Mike; SCHIEFER,
Nicholas; SHRAER, Alexander. **FoundationDB Record Layer: A
Multi-Tenant Structured Datastore.** In: *SIGMOD ’19*, 30
June–5 July 2019, Amsterdam. ACM. 16 pp. DOI
10.1145/3299869.3314039.
URL: https://www.foundationdb.org/files/record-layer-paper.pdf

Locus = **p. interna do PDF** (1–16 = ACM 1787–1802). Figs.
1–5 e Tables 1–2 conferidos no texto extraído. Não há
secção de microbench de laboratório.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract,
  §§1–12, Figs. 1–5, Tables 1–2, Apps. A–C. Referências
  [1]–[46] **não** foram relidas; CloudKit VLDB’18 [43],
  Percolator [41] e FDB [10] só pelo que *este* texto
  afirma (R041 e R043 já D4). Cascades [36] / Orca [45]
  não fichados.
- **Arquivo lido:** `research/fontes/R044_Chrysafis_2019_RecordLayer.pdf`
  + `.txt` + `.pages.txt`.
- Como foi lido: PDF local, seção a seção, nesta sessão.
  Números de Fig. 1 / Table 1–2 / §8.2 tirados do parágrafo
  que os comenta (não de um gráfico raster).
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 1–2).** Record Layer = biblioteca
open-source *em cima* de FoundationDB: records protobuf,
schema, índices, queries — “semantics similar to a
relational database” sem ser um RDBMS. Stateless; cada
tenant é um *record store* (base lógica isolada, inclusive
índices). CloudKit: **biliões** de bases independentes,
milhares de schemata, centenas de milhões de users.
Contribuições declaradas: layer relacional-like; record
store + técnicas para biliões de tenants; extensão
(índice/planner/schema); desenho *lightweight* sobre o KV.

**§2 Background FDB (p. 2–3).** Recapitula FDB: KV
ordenado, TX **strictly serializable** (MVCC reads + OCC
writes), GRV, 5 s de limite de TX, atómicos sem read
conflict, snapshot reads, key 10 KB / value 100 KB / TX
10 MB. CloudKit via RL: TX mediana **~7 KB**, p99
**~36 KB**. Tuple + directory layers. Simulação FDB: “more
than 250 million simulations … 1870 years and 3.5 million
CPU-hours” (p. 2) — é do *motor*, não do layer. “Currently,
the Record Layer is the most substantial layer built on
FoundationDB” (p. 3).

**§3 Princípios (p. 3–4).** Fig. 1: amostra 0.1% dos
record stores *privados* CloudKit — “a substantial
majority … fewer than 1 kilobytes of record data”; a
massa de bytes está nas bases grandes; públicos (TB) estão
*fora* da figura. Não partir o produto em “small vs big
data”. Princípios: (1) **stateless** — cursor = continuation
no cliente; (2) **streaming** — `ORDER BY` só se houver
índice; sem pool OLAP; (3) **schema flexível** — protobuf
aninhado/repetido; metadata *fora* do store (milhões de
DBs, um schema); (4) library embutida, async; (5)
extensível (índice + planner + schema).

**§4 Overview (p. 5–6).** Fig. 2: record store = subspace
contíguo (records + índices + header de versão). Tipos
interleaved no mesmo extent. Key expressions (App. A)
produzem tuples; podem *fan-out* em campos repetidos.
Records grandes são *split* em keys contíguas; um split
especial guarda o commit version. Continuations partem
scans que excedem 5 s. Prefetch para a cache RYW do
cliente FDB. `causal-read-risky` + cache de read version
(risco de stale; writes validam no commit). KeySpace API
≈ filesystem; directory layer encolhe prefixos.

**§5 Metadata (p. 6).** Metadata versionada, single-stream,
cacheada no cliente. Header do store = versão da app +
*storage format version* + *application version*. Índice
novo em tipo vazio = imediato; em tipo com dados = rebuild
em background (§6) se não couber numa TX. Protobuf:
campos novos, números nunca reutilizados.

**§6–7 Índices (p. 6–8).** Tese operacional: “Index
maintenance occurs in the same transaction as the record
change itself” (p. 6). Subspace por índice; range-clear
para dropar. Filtros → índice esparso. Save: se PK existe,
maintainers removem o velho, range-clear do record split,
inserir o novo, maintainers do novo; skip se o campo
indexado não mudou. Online build: estado **write-only**
depois **readable**; partido em várias TX.

Tipos: **VALUE** (campo→PK); **atomic mutation**
(COUNT / COUNT UPDATES / COUNT NON NULL / SUM / MAX|MIN
EVER) via atómicos FDB *sem* conflito — senão dois
updates concorrentes abortam no mesmo key de soma; aviso:
poucas keys quentes = hotspot no SS. **VERSION**: 12 B
(10 B do commit FDB + 2 B counter do layer); não vai no
protobuf; mapa PK→versão *adjacente* ao record. Índices
podem atravessar vários record types. RANK / TEXT no
App. B.

**§8 CloudKit (p. 8–10).** Fig. 3: subspace por user ×
record store por app → `(# users)×(# applications)` bases.
Schema CloudKit → metadata RL; zone prefixa a PK; índices
de sistema (quota). Table 1 (Cassandra → RL): TX de zona
→ **cluster**; concorrência zona → **record**; limite =
partição Cassandra (GBs) → **tamanho do cluster FDB**;
índice **eventual no Solr** → **transaccional no FDB**.
Isto desbloqueia TX interactivas via gRPC a outros
backends. Full-text personalizado com TEXT (prefix, n-gram
*n* keys em vez de \(O(n^2)\), proximity/phrase) —
transaccional, sem job Solr. Sync: Cassandra usava
update-counter por zona (serializa tudo); RL usa **VERSION
index**. Clusters FDB têm versões *não* correlacionadas →
**incarnation** (contador de mudas do user) + índice
`(incarnation, version)`; função-key para migrar o
update-counter legado como `(0, counter)`.

**§8.2 Isolamento (p. 10).** Query: **~38.3** keys lidas,
**~6.2** overhead (**~15%**). Point get: **~13.3** keys,
**~7.7** não-dados. Write: **~8.5** records/TX + **~34.5**
keys de índice ≈ **~4 writes/record**. Sem hash-join /
sort / agg em memória; limite de records/bytes →
continuation + throttle. “all clients make some progress
even when the system comes under stress” (p. 10).

**§9 Related (p. 10–11).** RDBMS shared-nothing: TX/índice
cross-shard caros. NoSQL: escala, semântica mínima.
NewSQL (Spanner+SQL [21, 29]). FDB = NewSQL *sem* modelo
— layers. TX-on-NoSQL (Percolator, Omid, Tephra, …): RL
*usa* as TX do FDB, não as reimplementa. Como Percolator,
é stateless. Multi-tenant: Salesforce Force.com [17]; RL
vai mais longe (record store = user×app). Search: Solr
eventual / Mongo sem garantia [5]. Planner: Selinger,
Cascades, Orca — “currently in the process” de Cascades
(App. C).

**§10 Lessons (p. 11–12).** (1) Layer FDB *funciona* à
escala CloudKit — “to our knowledge, the Record Layer is
deployed at a larger scale than any other FoundationDB
layer”. Async para esconder latência; não saturar a
network thread do cliente FDB. Conflict ranges manuais
são perigosos — encapsular em índices. Metadata: **não**
gerar schema a partir de descriptors protobuf em código
(não actualiza atomicamente N instâncias). Semântica
quase-relacional: um só extent (FKs CloudKit sem tabela);
`SELECT` por tipo = full scan *ou* índice; depois
emularam tabelas com prefixo de tipo na PK.

**§11–12 (p. 12).** Futuro: hot key do header; sort/join
limitados no streaming model; materialized views;
**SQL como layer em cima do Record Layer** (não no
kernel FDB). Conclusão: três observações — layer FDB
validado; extensão > one-size; tenants lógicos (user /
feature / entidade) escalam.

**Apps. A–C (p. 14–16).** Key expressions: `field`,
`nest`, `FanType` Concatenate/Fanout, `concat` (produto
cartesiano), `record type`, `version`, `function`,
`groupBy`, `KeyWithValue` (covering). RANK = skip-list
probabilística persistida em subspaces por nível +
contagens nos “fingers” (a ordem FDB *é* o finger);
Fig. 5. TEXT = postings `(prefix, token, pk) → offsets`;
*bunching* junta PKs no value. Table 2 / Moby Dick (233
docs ~5 KB): teórico 11.1 kB → 2.6 kB com bunch 20;
**prática 4.9 kB/doc**, bunch médio **~4.7**. Planner:
API fluente Java (não SQL texto); plans cacheáveis tipo
PREPARE; regras tipo Cascades; Memo/CBO ainda futuro.

## Tese central e argumento

Duas teses, uma de produto e uma de geometria:

1. **Produto:** um *layer stateless* sobre um KV com TX
   já strict-serializable dá records, índices
   transaccionais, queries e multi-tenant à escala
   CloudKit — sem meter SQL/schema no motor (p. 1–2,
   11–12). É o complementar de R043.
2. **Geometria:** índice = projecção KV **na mesma TX**
   que o record (p. 6). Solr-eventual (Table 1) é o que
   CloudKit *deixou*. Atomic SUM/COUNT só são
   conflict-free porque o FDB tem mutações sem read
   conflict (p. 3, 7). VERSION + incarnation substituem
   o update-counter que serializava a zona (p. 8–9).
   Continuations existem porque o FDB mata TX aos 5 s
   (p. 3, 5).

O paper **não** argumenta que Pedra (LSM local) deva
virar Record Layer. Argumenta que o sítio certo para
records/índices/SQL é *acima* do KV transaccional.

## Estrutura do texto

| § | p. | Conteúdo |
|---|---:|----------|
| Abstract / 1 | 1 | layer, biliões de DBs, CloudKit |
| 2 FDB | 2 | SSI, 5 s, atómicos, tuple/dir |
| 3 princípios | 3 | Fig. 1 sizes; stateless; streaming |
| 4 overview | 5 | Fig. 2 store; continuations |
| 5 metadata | 6 | versões; rebuild |
| 6–7 índices | 6 | mesma TX; VALUE / atomic / VERSION |
| 8 CloudKit | 8 | Fig. 3; Table 1; sync; TEXT |
| 8.2 isolation | 10 | 38.3 / 15% / 4 writes |
| 9 related | 10 | NewSQL, Percolator, Solr, Cascades |
| 10 lessons | 11 | async; conflict ranges; um extent |
| 11–12 | 12 | hotspots; SQL-on-RL; tenants |
| A–C | 14 | key expr; RANK/TEXT; planner |

## Conceitos-chave

- **Record store.** Base lógica = subspace contíguo
  (records + índices + header). Unidade de isolamento e
  de muda entre clusters (p. 2, 5).
- **Stateless layer.** Estado no FDB ou continuation no
  cliente. Qualquer servidor serve o próximo request
  (p. 4).
- **Key expression.** Função record → 1..N tuples; pode
  fan-out. Define PK e índice (App. A).
- **Index maintainer.** Plugin que actualiza o índice no
  save/delete; built-in + cliente (p. 7).
- **Write-only → readable.** Índice online não é
  consultável até o rebuild acabar (p. 7).
- **Atomic mutation index.** Agregado numa key, UPDATE
  via atómico FDB sem read conflict (p. 7).
- **VERSION index.** 12 B monótono por cluster; sync
  ordenado; + incarnation cross-cluster (p. 8–9).
- **Continuation.** Cursor serializado; parte o trabalho
  pelas TX de 5 s e pelo throttle (p. 4–5, 10).
- **Incarnation.** Contador de mudas do tenant; prefixa
  o índice de sync (p. 9).
- **Single extent.** Todos os tipos no mesmo keyspace
  (FKs CloudKit sem tabela) (p. 12).

## Citações relevantes

1. "The FoundationDB Record Layer is an open source library that provides a record-oriented data store with semantics similar to a relational database implemented on top of FoundationDB, an ordered, transactional key-value store." (p. 1)
2. "CloudKit uses the Record Layer to host billions of independent databases, many with a common schema." (p. 1)
3. "the Record Layer allows creating isolated logical databases for each tenant—at Apple, it is used to manage billions of such databases—all while providing familiar features such as transactional index maintenance." (p. 2)
4. "the layer is completely stateless, so scaling the compute service is as easy as launching more stateless instances." (p. 2)
5. "FoundationDB imposes a 5 second transaction time limit, which the Record Layer compensates for using techniques described in Section 4." (p. 3)
6. "Atomic operations occur within a transaction like other operations, but they do not create read conflicts" (p. 3)
7. "the median and 99th percentile transaction sizes are approximately 7KB and 36KB, respectively." (p. 3)
8. "Notice that a substantial majority of record stores contain fewer than 1 kilobytes of record data." (p. 4, Fig. 1)
9. "The Record Layer does not maintain any state about the position of a cursor in memory; instead, the context needed to advance a cursor in a future request is serialized and returned to the client as a continuation." (p. 4)
10. "it supports ordered queries (as in SQL's ORDER BY clause) only when there is an available index supporting the requested sort order." (p. 4)
11. "unlike tables in a relational database, all record types within a record store are interleaved within the same extent." (p. 5)
12. "Index maintenance occurs in the same transaction as the record change itself, ensuring that indexes are always consistent with the data." (p. 6)
13. "indexes begin in a write-only state where writes maintain the index but it cannot be used to satisfy queries." (p. 7)
14. "the index is updated using FoundationDB's atomic mutations (e.g., the ADD mutation for the SUM index), which do not conflict with other mutations." (p. 7)
15. "The first 10 bytes are assigned by the FoundationDB servers upon commit, and only the last 2 bytes are assigned by the Record Layer" (p. 8)
16. "With the Record Layer, user-defined secondary indexes are maintained transactionally with updates, so all queries return the latest data." (p. 9)
17. "The VERSION sync index maps (incarnation, version) pairs to changed records, sorting the changes first by incarnation, then version." (p. 9)
18. "A query operation, which returns all records that match a given query, reads an average of ~38.3 keys, of which ~6.2 are not the records or index entries themselves, for a total overhead of ~15%." (p. 10)
19. "On average, a CloudKit transaction writes ~8.5 records and makes ~34.5 key writes associated with indexes, so the total index overhead is approximately ~4 writes per record." (p. 10)
20. "Today, the Record Layer does not provide the ability to perform in-memory query operations, such as hash joins, grouping, aggregation, or sorts." (p. 10)
21. "Like Google Percolator [41] the Record Layer is completely stateless, keeping all its metadata in the underlying FoundationDB data store." (p. 11)
22. "The Record Layer's success at Apple validates the usefulness of FoundationDB's \"layer\" concept" (p. 11)
23. "Bugs due to incorrect manual conflict ranges are very hard to find, especially when mixed with business logic." (p. 11)
24. "A higher level \"SQL layer\" could be implemented as a separate layer on top of the Record Layer without needing to work around choices made by lower-level layers." (p. 12)
25. "this index required ~4.9 kilobytes per document because not every bunch is actually filled. In fact, the average bunch size was ~4.7" (p. 16; Table 2: 11.1 kB → 2.6 kB teórico)

## Diálogo teórico

- **FoundationDB [10] / R043** — o KV; este paper é a
  layer canónica. Limites 5 s / 10 MB / atómicos
  *vazam* para o desenho do layer.
- **Percolator [41] / R041** — stateless sobre KV (p. 11).
  Percolator *implementa* SI+2PC no Bigtable; RL *consome*
  SSI do FDB. Não reabre L22/L23.
- **CloudKit [43]** — o cliente; Cassandra+Solr → RL
  (Table 1). Paper de 2018 **não** relido aqui.
- **Cassandra LWT [3, 6]** — CAS no update-counter da
  zona; sem concorrência intra-zona.
- **Solr [1] / Mongo text [5]** — search eventual; RL
  recusa isso para índice de user.
- **Spanner [29] + Spanner SQL [21]** — NewSQL com
  modelo no motor; FDB recusa o modelo.
- **Salesforce Force.com [17]** — multi-tenant por app;
  RL isola *user×app* no keyspace.
- **Omid / Tephra / Megastore / CloudTPS** — TX *em cima*
  de NoSQL; RL não precisa.
- **Selinger [42], Cascades [36], Orca [45]** — planner;
  App. C ainda a caminho do Memo/CBO.
- **Cormen skip-list [30]** — RANK (Fig. 5).

## Relação com Pedra / Montanha

R043 fechou o motor FDB. Este paper fecha o *andar de
cima* que RFC-0010 e L9 já apostam. Pedra **já** tem o
canário (`pedradb-index`: row + 2 idx na mesma TX;
`RecordTable` em `montanha-fdb-recipes` — “not Apple
Record Layer”). `pedradb-sql` é prefixo de tabela, não
protobuf + Cascades.

| Record Layer (paper) | Pedra / Montanha | Ledger |
|----------------------|------------------|--------|
| KV + TX no motor; records/SQL na layer | L9, RFC-0010, `-sql`/`-dcs`/`-fold` | **L9 `SHIP` src+=ficha R044** (p. 2, 11–12) |
| Índice = projecção KV na **mesma TX** | `pedradb-index` + `RecordTable` seed | **L31 `SHIP`**: nunca Solr-eventual; o canário *é* a geometria |
| Protobuf + record store + Cascades + TEXT/RANK no *core* | kernel = bytes; sql = `t/{key}` | **L32 `REFUSE`** embutir o produto RL no kernel (nem como default SQL) |
| VERSION index p/ sync | fold = LocalApplied (RFC-0024); sem índice de versão | **L33 `MEASURE`**: só se um produto nomear “scan since V” mais barato que log-scan |
| Atomic SUM/COUNT via mutação FDB sem conflito | OCC local serializa o agregador | **L34 `REFUSE`** atómicos estilo FDB no kernel |
| Continuations por limite 5 s | L30 já recusou a janela no kernel | **não** nova linha. Pattern útil *se* houver servidor SQL stateless; não P0 |
| Single extent / FKs sem tabela | sql tem prefixo-por-tabela (o próprio paper emulou isto, p. 12) | não copiar |
| Incarnation p/ muda de cluster | não há CloudKit-move | não P0 |

- **Não implementar** Record Layer Java, planner Cascades,
  TEXT/RANK, metadata store, protobuf no MANIFEST.
- **Teste se roubássemos L33:** um soak de CHANGELOG/fold
  em que `scan` desde o cursor custa >2× um índice
  `(seq, pk)` escrito na mesma TX que o apply — e o DST
  de crash a meio *não* deixa half-index (já o W3
  `index-tx-crash`).
- **Falsificaria L32:** um produto Caixote que *precise*
  de biliões de record stores protobuf *e* medir que
  vender RL é mais barato que o seed. Mesmo assim o
  sítio é layer, não `pedradb-core`.
- **Falsificaria L34:** um agregado quente (quota, SUM)
  no store em que RMW aborta >X% *e* um atómico
  sem read-set no WAL/Raft se justificar. O paper não
  mede isso no nosso hardware.

Fichas irmãs: R043 FDB (D4); R041 Percolator (D4);
R048 TiDB (D4 — SQL *no* cluster, outra geometria);
R010 library local.

## Avaliação crítica

- **Industrial experience, não eval de sistemas.** Não
  há YCSB, não há Fig. de throughput de laboratório.
  Os números de §8.2 são médias CloudKit (keys por
  request), não ops/s.
- **Fig. 1 enviesa para o pequeno:** exclui públicos TB
  (caption p. 4). A tese “um sistema para small e big”
  apoia-se nisto + no texto, não num histograma dos
  grandes.
- **Table 1 é qualitativa.** Cassandra→RL é o ganho de
  *semântica* (TX cluster-wide, índice transaccional),
  não um speedup medido.
- **TEXT / Table 2 = Moby Dick, 233 docs.** Bunch médio
  4.7 vs máx. 20: o teórico 2.6 kB não se materializa.
  Não é um benchmark de search.
- **Planner Cascades é futuro** (p. 11, App. C). CloudKit
  já estende o planner ad-hoc. Não citar este paper
  como “RL tem CBO”.
- **Limites FDB vazam.** Continuations, write-only
  rebuild, split de records, 7/36 KB de TX — tudo
  consequência de 5 s / 10 MB / 100 KB (R043 L30).
- **Hotspot do header e das keys de SUM** (p. 7, 12)
  são admitidos, sem número.
- **Não medem:** LSM vs B-tree no SS (é FDB/SQLite);
  custo do índice vs Solr em latência; abort rate do
  VERSION vs update-counter; Pedra-class local embed.

## Palavras-chave

record-layer; foundationdb; layers; multi-tenant;
record-store; protobuf; indexes; same-tx; atomic-mutation;
version-index; continuation; cloudkit; cascades; text-index

## Fontes

**Primária:** `research/fontes/R044_Chrysafis_2019_RecordLayer.pdf`
(SIGMOD ’19, PDF foundationdb.org) + `.txt` +
`.pages.txt`.

**Secundárias (não citadas como se fossem este
paper):** RFC-0010; `docs/grail-plan-build-databases-on-pedradb.md`;
`docs/montanha-fdb-recipes.md`; ficha R043; ficha R041;
`crates/pedradb-index`; `crates/pedradb-sql`;
`montanha-fdb-recipes::RecordTable`.
