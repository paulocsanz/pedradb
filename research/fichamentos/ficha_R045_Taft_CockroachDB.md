# Fichamento: R045 — CockroachDB (SIGMOD’20)

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R045`

## Referência

TAFT, Rebecca; SHARIF, Irfan; MATEI, Andrei; VANBENSCHOTEN,
Nathan; LEWIS, Jordan; GRIEGER, Tobias; NIEMI, Kai; WOODS,
Andy; BIRZIN, Anne; POSS, Raphael; BARDEA, Paul; RANADE,
Amruta; DARNELL, Ben; GRUNEIR, Bram; JAFFRAY, Justin;
ZHANG, Lucy; MATTIS, Peter. **CockroachDB: The Resilient
Geo-Distributed SQL Database.** In: *SIGMOD ’20*, 14–19 June
2020, Portland, OR. ACM. 17 pp. DOI 10.1145/3318464.3386134.
URL (PDF livre): https://www.cockroachlabs.com/pdf/cockroachdb-the-resilient-geo-distributed-sql-database-sigmod-2020.pdf

Locus = **p. interna do PDF** (1–17 = ACM 1493–1506 + refs).
Figs. 1–7, Table 1 e Algorithms 1–2 conferidos no texto
extraído. Fig. 4/5/7 barras: números só do parágrafo.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract,
  §§1–9, Figs. 1–7, Table 1, Algs. 1–2. Referências [1]–[79]
  **não** foram relidas. Spanner [4, 17], F1 [52], Calvin
  [66], TiDB [67], FDB [24] e Record Layer [10] só pelo que
  *este* texto afirma (R041, R043, R044, R048 já D4). Papers
  CRDB 2022/2025/2026 (R046/R047/R056) **não** lidos.
- **Arquivo lido:** `research/fontes/R045_Taft_2020_CockroachDB.pdf`
  + `.txt` + `.pages.txt`.
- Como foi lido: PDF local, seção a seção, nesta sessão.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 1–2).** CRDB = SQL DBMS *from the
ground up* para OLTP global: HA, consistência forte,
commodity (NTP chega). Requisitos do caso (empresa real
EU/AU/US): GDPR domicílio, dados perto do user, sobreviver
região, **SQL + serializable**. Três eixos: (1) ≥3 réplicas
+ recovery automático; (2) partição geo + placement fino;
(3) TX geo-distribuídas **sem hardware especial**. Extra:
optimizer, distSQL, schema online, backup, JSON. Código
BSL → Apache aos 3 anos; cloud-neutral.

**§2 Arquitectura (p. 2–4).** Shared-nothing; qualquer
nó é storage+compute. Camadas *num* processo:

1. **SQL** — parser / Cascades / execução. Em geral não
   vê partições (KV monolítico); algumas queries quebram
   a abstração (§5).
2. **Transactional KV** — atomicidade + isolamento.
3. **Distribution** — keyspace ordenado; **Ranges ~64 MiB**
   (split/merge por tamanho *e* por load). Índice de dois
   níveis em system Ranges, cacheado.
4. **Replication** — Raft **por Range** (Multi-Raft).
5. **Storage** — “we rely on RocksDB … treat as a black
   box” (p. 3). **Não** avaliam Pebble neste paper.

**§2.2 Raft + lease (p. 3).** Unidade = *command* (edits
ao engine). **Leaseholder** (em geral o leader): único
que serve reads authoritativos e propõe writes — reads
*bypass* Raft. Leases de user Ranges ligadas à liveness
do nó (heartbeat **4.5 s** num system Range); system
Ranges usam expiration **9 s**. Aquisição de lease vai
no log Raft com a lease que o requester *acha* válida
→ leases disjuntas no tempo. Falha curta: Raft se
maioria viva; catch-up por snapshot ou log. Falha
longa: re-replicar. Gossip para liveness/métricas.

**§2.3 Placement (p. 3–4).** Manual (atributos +
constraints no schema) e automático (failure domains +
heurísticas). Políticas: Geo-Partitioned Replicas (read
*e* write locais; região abaixo = dados da região
indisponíveis); Geo-Partitioned Leaseholders (read
local, write cross-region, sobrevive região); Duplicated
Indexes (read local, WA maior).

**§3 TX (p. 4–7).** MVCC + ordenação por timestamp →
serializable. Gateway = coordenador (Alg. 1). Timestamp
inicial = now(); pode avançar. **Write Pipelining:**
ops em keys distintas sem esperar replicate.
**Parallel Commits:** status `staging` torna o commit
condicional a *todos* os writes replicados — staging
replica *em paralelo* com os writes em falta; 1 RTT
de consenso no caso comum. TLA+ [14]: todo staging
acaba committed ou aborted; committed fica. Fig. 2
(3 regiões, 1 row + N índices): até **+72%** throughput
e **−47%** p50 vs 2PC ingénuo.

Leaseholder (Alg. 2): verifica lease; latches em
`op ∪ deps`; escreve só depois do timestamp do último
read na key; avalia *sem* mutar; responde antes do
Raft se não for commit; depois replica e aplica.

**§3.2 Intents (p. 6).** Write = intent (MVCC +
ponteiro para *transaction record* no Range do primeiro
write: pending / staging / committed / aborted). Reader
segue o record: committed → valor; aborted → ignora;
pending → espera; staging → tenta abortar um write
ainda não replicado, senão o TX *já* commitou.
Heartbeat do record em TX longas. Coordenador morto
→ record expira → abort.

**§3.3–3.4 Conflitos / refresh (p. 6).** WW: espera
intent mais velho ou avança timestamp. WR: espera
intent mais velho. RW: writer avança para além do
read. Deadlock detector distribuído. **Read refresh:**
provar que o read-set não mudou em `(ta, tb]` (como
SSI do Postgres [8]); falso positivo permitido. Se o
cliente já viu resultado → retry do cliente.

**§3.5 Follower reads (p. 6–7).** `AS OF SYSTEM TIME`
no passado. Leaseholder emite **closed timestamp**
(abaixo do qual não aceita mais writes); ~**2 s** atrás
do now. Followers só servem se tiverem o prefixo do
log. Encaminha para a réplica mais próxima.

**§4 Relógios (p. 7–8).** **HLC** [20] (físico +
Lamport). Offset máximo default **500 ms**. Propriedades:
causalidade nas mensagens; leases disjuntas (handoff
cooperativo via HLC; não-cooperativo espera
max_offset); monotonia no restart (espera max_offset
no boot); auto-estabilização sem garantia. **Não** é
strict serializable: TX em keys disjuntas podem
inverter o tempo real. **Uncertainty interval**
`[commit_ts, commit_ts+max_offset]` → single-key
linearizability *se* os relógios cabem no offset.
Skew para além do bound: **isolamento Raft+lease
mantém-se**; linearizability single-key *pode*
quebrar (stale read). Nó que excede 80% do offset
vs maioria **suicida-se**.

Safeguards de lease sob skew: (1) lease tem
`[start,end]`; (2) cada write no Raft leva o seq da
lease — se a lease activa mudou, reject.

**§5 SQL (p. 8–10).** Postgres dialect + wire. Tabela
= índices ordenados (primário = PK→cols; secundário =
idx→PK[+stored]). Hash indexes anti-hotspot. Optimizer
Cascades, **>200** regras, DSL **Optgen** → Go.
Distribution-aware (filtros de partição estáticos;
custo por proximidade da réplica). Execução:
gateway-only ou distributed (**só reads**). DistSQL:
TableReader por nó, pushdown, hash-shuffle (Fig. 3).
Volcano row-at-a-time + vectorizado estilo
MonetDB/X100: **>100×** em operadores isolados, até
**4×** TPC-H. Schema online = protocolo F1 [52]
(no máximo 2 versões sucessivas; índice secundário
passa por 2 estados intermédios).

**§6 Eval (p. 10–12).** CRDB **v19.2.2** (TPC-C em
**v19.2.0**).

- Fig. 4 Sysbench: txns/s/vCPU ~constante de 6 a
  **1728** vCPUs (vertical c5d + horizontal até 48×
  c5d.9xlarge, 3 AZ us-east-1, ~38 GB no maior).
- Fig. 5 TPC-C: replicação corta até **48%** (RF=3)
  ou **57%** (RF=5); TX remotas ainda **−46%**;
  *mesmo assim* escala linear. Até 10k warehouses /
  800 GB (n1-standard-4).
- Table 1: 100k warehouses = **50 B rows / 8 TB**,
  **1 245 462 tpmC**, **98.8%** efficiency, 81×
  c5d.9xlarge, NewOrder p90 **486.5 ms**. 10k:
  124 036 tpmC / 96.5% / 15 nós / p90 436 ms. 1k:
  12 474 / 97% / 3 nós / p90 39.8 ms. Aurora 10k:
  9 406 tpmC / **7.3%** (single-master).
- Fig. 6 (9× n1-standard-4, 3 regiões US): só
  *geo-partitioned leaseholders* sobrevive região;
  replicas-locais têm p90 melhor em estável.
- Fig. 7 YCSB vs Spanner: CRDB mais throughput na
  maioria; Workload A (update+zipf) **não** escala
  (contenção). Latência sob load leve menor que
  Spanner (eles atribuem ao *commit-wait*).

**§7 Lessons (p. 12–13).** (1) Raft “fácil” não é:
centenas de milhares de grupos → coalescer heartbeats
*por nó* + pausar grupos idle; **Joint Consensus**
para mover réplica sem janela de 2 ou 4 réplicas.
Recomendam Joint Consensus a todo Raft de produção.
(2) SNAPSHOT removido: virar SI sem write-skew
exigiria locks pessimistas *também* no caminho
SERIALIZABLE; SI ficou alias. (3) Compat Postgres:
drivers impulsaram adopção *e* fricção (retry MVCC,
paging). (4) Upgrade rolling: **evaluate-then-propose**
(propõe o *efeito*, não o request) para réplicas
mistas não divergirem. (5) Follow-the-Workload quase
não usado — operadores querem placement previsível.

**§8–9 (p. 13–14).** Relacionam Spanner (strict SI +
commit-wait + TrueTime), Calvin/Fauna/SLOG (precisa
R/W set up front → sem SQL conversacional),
H-Store/VoltDB (mau em cross-partition), Aurora
(log no storage partilhado; um master), TiDB (MySQL
+ HTAP, **só SI**, “not optimized for geo”), FDB +
Record Layer (KV SSI; SQL subset na layer). Conclusão:
1 RTT no caso comum; consenso só nas partições
*escritas*. Futuro: storage redesenhado, geo-aware
optimizer, serverless / disagg — **não medido**.

## Tese central e argumento

Três teses, não uma:

1. **Produto:** SQL conversacional + serializable +
   geo + commodity clocks, num único binário
   shared-nothing (p. 1–2). É o *contrário* da
   identidade FDB/Pedra (KV+TX no núcleo; SQL é
   layer — L9).
2. **Replicação:** Multi-Raft por Range ~64 MiB +
   leaseholder que *salta* o Raft nos reads (p. 3).
   É a geometria que RFC-0017 já escolheu contra o
   Sequencer FDB (L29).
3. **TX:** intents + record + Parallel Commits +
   HLC/uncertainty 500 ms → serializable e
   single-key linearizability, **não** strict
   serializable (p. 7–8). Não é Percolator (R041)
   nem OCC+MVCC do FDB (R043).

O paper **não** argumenta que Pedra deve virar
CRDB. Argumenta que SQL-on-Multi-Raft funciona à
escala TPC-C 8 TB *neste* protocolo e *neste*
hardware.

## Estrutura do texto

| § | p. | Conteúdo |
|---|---:|----------|
| Abstract / 1 | 1 | geo OLTP; SQL+SSI; Fig. 1 |
| 2.1 layers | 2 | SQL / KV / ranges / Raft / Rocks |
| 2.2–2.3 | 3 | lease 4.5 s; 3 políticas geo |
| 3 TX | 4 | Alg. 1–2; Fig. 2 Parallel Commits |
| 3.2–3.5 | 6 | intents; refresh; closed ts ~2 s |
| 4 clocks | 7 | HLC 500 ms; uncertainty |
| 5 SQL | 8 | Cascades; DistSQL; F1 schema |
| 6 eval | 10 | Fig. 4–7; Table 1 TPC-C |
| 7 lessons | 12 | Raft chatter; Joint; no SI |
| 8–9 | 13 | Spanner/Calvin/TiDB/FDB |

## Conceitos-chave

- **Range.** Chunk ordenado ~64 MiB; um grupo Raft
  (p. 2–3). Termo CRDB (TiKV diz Region).
- **Leaseholder.** Réplica que lê sem Raft e propõe
  writes (p. 3). ≠ learner (R048).
- **Write intent / transaction record.** Valor
  provisório + status atómico (p. 6).
- **Parallel Commits.** Status `staging`; 1 RTT (p. 5).
- **Read refresh.** Validar read-set após bump de ts
  (p. 6). SSI-like.
- **HLC + uncertainty interval.** 500 ms default
  (p. 7). Single-key linearizability, não strict SI.
- **Closed timestamp.** Barreira de writes para
  follower reads (~2 s) (p. 7).
- **Joint Consensus.** Membership atómica do paper
  Raft (p. 12).
- **Evaluate-then-propose.** Propõe o efeito, não o
  request (p. 13).

## Citações relevantes

1. "CockroachDB is a scalable SQL DBMS that was built from the ground up to support these global OLTP workloads while maintaining high availability and strong consistency." (p. 1)
2. "CRDB uses range-partitioning on the keys to divide the data into contiguous ordered chunks of size ~64 MiB" (p. 2)
3. "At the time of writing, we rely on RocksDB [54], which is well-documented elsewhere, and which we treat as a black box throughout the paper." (p. 3)
4. "Because all writes go through the leaseholder, reads can bypass networking round trips required by Raft without sacrificing consistency." (p. 3)
5. "nodes heartbeat a special record in a system Range every 4.5 seconds. System Ranges in turn use expiration based leases which must be renewed every 9 seconds." (p. 3)
6. "Combined, they allow many multi-statement SQL transactions to complete with the latency of just one round of replication." (p. 4)
7. "Parallel Commits improves throughput by up to 72% and reduces p50 latency by up to 47% when the table has one or more secondary indexes" (p. 5, Fig. 2)
8. "An intent is a regular MVCC KV pair, except that it is preceded by metadata indicating that what follows is an intent." (p. 6)
9. "This process is equivalent to detecting the rw-antidependencies that PostgreSQL tracks for its implementation of SSI" (p. 6)
10. "closed timestamps typically trail current time by ~2 seconds" (p. 7)
11. "This offset configuration defaults to a conservative value of 500 ms." (p. 7)
12. "Note that CRDB does not support strict serializability because there is no guarantee that the ordering of transactions touching disjoint key sets will match their ordering in real time." (p. 7)
13. "If any node exceeds the configured maximum offset by more than 80% compared to a majority of other nodes, it self-terminates." (p. 8)
14. "CRDB scales to support up to 100,000 warehouses, corresponding to 50 billion rows and 8 TB of data, at near-maximum efficiency." (p. 11, Table 1: 1 245 462 tpmC, 98.8%)
15. "Amazon Aurora … achieves 7.3% efficiency with 10,000 warehouses" (p. 11)
16. "the overhead of replication can reduce throughput by up to 48% for three replicas or 57% for five replicas, and distributed transactions may further reduce throughput by up to 46%" (p. 11)
17. "a large CRDB deployment may need to maintain hundreds of thousands of consensus groups (one per Range), this communication becomes expensive." (p. 12)
18. "we recommend that all production-grade Raft-based systems use Joint Consensus instead." (p. 13)
19. "To avoid this pessimization of the common path, we opted to eschew true support for SNAPSHOT, keeping it as an alias to SERIALIZABLE instead." (p. 13)
20. "we moved the evaluation stage first, and now propose the effect of an evaluated request, rather than the request itself." (p. 13)
21. "TiDB [67] is an open-source distributed SQL DBMS … These systems are not optimized for geo-distributed workloads and only support snapshot isolation." (p. 14)
22. "Unlike systems that require global consensus or a single master region for ordering multi-partition transactions, CRDB requires consensus only from partitions written in the transaction." (p. 14)

## Diálogo teórico

- **Raft [46]** — por Range; lessons §7.1 (chatter,
  Joint Consensus).
- **Spanner [4, 17]** — strict SI + TrueTime +
  commit-wait. CRDB recusa o hardware; paga retries
  sob contenção (p. 13–14).
- **HLC [20] / Lamport [37]** — relógio.
- **Postgres SSI [8, 49]** — refresh ≈ antidependências.
- **F1 [52]** — DistSQL + schema online.
- **Calvin [66] / SLOG / Fauna** — deterministic;
  sem SQL conversacional.
- **Aurora [72]** — redo no storage; um master
  (Table 1).
- **TiDB [67] / R048** — “só SI, não geo” *neste*
  related (2020). R048 é HTAP 2020; não reabre L24.
- **FDB [24] + Record Layer [10] / R043–R044** —
  KV SSI + SQL na *layer*. CRDB mete SQL no mesmo
  binário.
- **Percolator** — **não** citado no related. Intents
  CRDB ≠ Figs. 4–6 de R041 (sem oracle, sem lock
  column).

## Relação com Pedra / Montanha

Pedra = LSM local + OCC. Store = intents FDB-shaped +
2PC. `pedradb-raft` = **um** grupo. SQL = prefixo
(`pedradb-sql`). Fold = LocalApplied, não leaseholder
(L8/L24). RFC-0017: produto FDB-shaped; Multi-Raft
como ferramenta; Parallel Commit–class *cross-range*
só com gate P1.

| CRDB (paper) | Pedra / Montanha | Ledger |
|--------------|------------------|--------|
| SQL+KV+Raft no mesmo binário | L9: Pedra local; SQL é crate | **L9 `SHIP` src+=ficha R045** (p. 2–3, 8). Não clonar o *produto* CRDB |
| Multi-Raft 2f+1 por Range 64 MiB | `pedradb-raft` single-group; RFC-0017 já recusou Sequencer | **L29 confirmado.** **L39 `MEASURE`**: split/merge + leaseholder quando o store tiver >1 grupo. Gate: produto nomeia o 2.º grupo |
| Leaseholder lê sem Raft | fold *não* é isto (L24) | **não** no fold. Lease é do *store* se L39 |
| Closed ts ~2 s / follower read | L24 já recusou read-index no fold | **confirma L24.** Stale 2 s ≠ LocalApplied |
| Intents + Parallel Commits + HLC 500 ms | kernel OCC; store 2PC FDB; sequence sem wall-clock (L30) | **L40 `REFUSE`** no kernel. Store: Parallel Commits só no gate RFC-0017 P1 (cross-range) — **não** SHIP agora |
| Cascades / Optgen / vectorizado | `pedradb-sql` é `t/{key}` | **L32 cobre.** Não nova linha |
| Joint Consensus + heartbeat coalesce | um grupo hoje | **parte de L39**, não linha extra |
| Evaluate-then-propose | apply já é efeito no log | prática existente; não ledger |
| SNAPSHOT alias SSI | OCC kernel > SI (L22) | confirma; não reabre |
| Rocks black box → “storage redesenhado” | Pedra LSM | **não** importar Pebble. Paper **não** mede Pebble |

- **Não implementar:** SQL Postgres, Cascades, HLC no
  kernel, geo-partition policies, Follow-the-Workload,
  vectorized engine, 64 MiB ranges no core.
- **Teste se L39:** dois grupos Raft, split de um
  prefixo, lease num lado, get no leaseholder sem
  majority round-trip; DST: dois nós a *acharem-se*
  leaseholder → um reject (paper p. 8, safeguard 2).
- **Falsificaria L40:** um 2PC cross-range do store
  cujo p50 é >2× o Parallel Commits do paper *e* o
  TLA+/DST do staging sob crash do coordenador cabe
  no nosso `FailingEnv`. Mesmo aí o sítio é o store,
  não `pedradb-core`.
- **Falsificaria L9:** um produto Caixote que *precise*
  do dialecto PG + DistSQL *dentro* do kernel. O paper
  não mostra isso; mostra um DBMS.

Fichas irmãs: R043 FDB (D4, outra geometria de
commit); R044 Record Layer (SQL *acima*); R048 TiDB
(Multi-Raft + Percolator no engine; learner ≠
leaseholder); R041 Percolator (intents diferentes);
R052 Raft (listed).

## Avaliação crítica

- **OLTP geo, AWS/GCP 2019–20, CRDB 19.2.** Não é
  HTAP (R048 p. 12 “CRDB no HTAP” confirma-se *neste*
  paper). Vectorizado 4× TPC-H ≠ TiFlash.
- **Table 1 vs Aurora** é single-master Aurora 10k —
  comparação fácil. Multi-master Aurora “not
  published”.
- **YCSB vs Spanner:** hardware Spanner opaco; eles
  alinham preço de 3× n2-standard-8 a “1 nó” Spanner.
  Workload A admite que **não** escala.
- **Fig. 2 Parallel Commits** = 3 servidores / 3
  regiões / microbench de índices — não TPC-C.
- **HLC 500 ms** é conservador; Spanner é ms com
  GPS. O paper não mede quantos refreshes/aborts o
  500 ms custa em WAN.
- **Rocks black box.** WA/compact do motor *não*
  entram. “Pebble” no hook do catálogo é o *futuro*
  do §9, não o eval.
- **Não medem:** DST/simulação (contrastar R043);
  learner (não há); Pedra-class embed; upgrade
  evaluate-then-propose com número de bugs
  evitados.

## Palavras-chave

cockroachdb; multi-raft; range; leaseholder; hlc;
uncertainty; parallel-commits; write-intent;
serializable; follower-reads; joint-consensus;
distsql; geo-partition

## Fontes

**Primária:** `research/fontes/R045_Taft_2020_CockroachDB.pdf`
(SIGMOD ’20, PDF cockroachlabs.com) + `.txt` +
`.pages.txt`.

**Secundárias (não citadas como se fossem este
paper):** RFC-0017; `docs/montanha-vs-foundationdb.md`;
`pedradb-raft`; `pedradb-sql`; fichas R043, R044,
R048, R041. R046/R047/R056 **não** lidos.
