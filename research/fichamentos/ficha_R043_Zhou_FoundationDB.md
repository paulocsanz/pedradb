# Fichamento: R043 — FoundationDB (SIGMOD’21)

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R043`

## Referência

ZHOU, Jingyu; XU, Meng; SHRAER, Alexander; NAMASIVAYAM, Bala;
MILLER, Alex; TSCHANNEN, Evan; ATHERTON, Steve; BEAMON,
Andrew J.; SEARS, Rusty; LEACH, John; ROSENTHAL, Dave;
DONG, Xin; WILSON, Will; COLLINS, Ben; SCHERER, David;
et al. **FoundationDB: A Distributed Unbundled Transactional
Key Value Store.** In: *SIGMOD ’21*, 20–25 June 2021 (virtual).
14 pp. ACM. DOI 10.1145/3448016.3457559.
URL: https://www.foundationdb.org/files/fdb-paper.pdf

Locus = **p. interna do PDF** (1–14). Figs. 1–10 e Algorithm 1
conferidos no texto extraído.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract, §§1–8,
  Figs. 1–10, Algorithm 1, §3 geo, §4 simulation, §5 eval,
  §6 lessons. Referências [1]–[71] **não** foram relidas;
  Percolator [55] e Record Layer [28] só pelo que o texto
  afirma (R041 fichada; R044 ainda listed).
- **Arquivo lido:** `research/fontes/R043_Zhou_2021_FoundationDB.pdf`
  + `.txt` + `.pages.txt`.
- Como foi lido: PDF local, seção a seção, nesta sessão.
  Fig. 7/8/10 números tirados do parágrafo que as comenta.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 1–2).** FDB (2009) = ordered KV +
transações **strictly serializable** em todo o keyspace.
Arquitectura **unbundled**: transaction system in-memory
+ storage distribuído + configuração (Paxos). Feature set
mínimo — sem SQL, schema, índices; isso fica nas
**layers** (Record Layer, JanusGraph, CouchDB). Apple /
Snowflake / VMware. A tese de engenharia: **primeiro**
um simulador determinístico (rede+disco+falhas num
só processo); só depois a base. Sem isso, bugs de
interleaving corrompem dados meses depois.

**§2.1 Princípios (p. 2–3).** (1) Divide-and-conquer:
write path (TS) ≠ read path (SS); papéis
heterogéneos. (2) *Make failure a common case*: o
TS **desliga-se** ao detectar falha e recupera por
reconfiguração — um caminho, bem testado. (3) Fail
fast / recover fast; MTTR produção **< 5 s** (§5.3).
(4) Simulation testing.

**§2.2 API (p. 3).** get/set, getRange, clear prefix.
Writes buffered no cliente; RYW no commit. Limites:
key 10 KB, value 100 KB, TX **10 MB**. Snapshot
reads relaxam isolation.

**§2.3 Arquitectura (p. 3–4).** Control plane:
Coordinators (Active Disk Paxos [27]) elegem
ClusterController → Sequencer, DataDistributor,
Ratekeeper. Data plane: **TS** (Sequencer, Proxies,
Resolvers — *stateless*) + **LS** (LogServers =
filas persistentes) + **SS** (StorageServers,
B-tree SQLite modificado por processo). Reads
escalam com SS; writes com Proxies/Resolvers/Logs.
MVCC vive no SS, não no TS (≠ Deuteronomy).
Bootstrap sem ZooKeeper (apagaram-no em 2010
depois de 2 bugs, §6.2). Reconfiguração = nova
*epoch* do Sequencer.

**§2.4 Transacções (p. 4–6).** Cliente pede *read
version* ao Proxy→Sequencer (≥ qualquer commit
anterior). Reads aos SS nessa versão. No commit:
commit version (1 M versões/s), Resolvers
particionados verificam read-write (Algorithm 1,
Skiplist versionada), depois persistir nos
LogServers **k = f+1**. Commit = todos os k
responderam. SS puxam o log **em background**
(não está no commit path). Read-only commita
**local** no cliente. Produção: conflito **< 1%**
(§2.4.2); CloudKit 0.73% (§5.1).

Strict serializability = OCC + MVCC; o Sequencer
define a ordem (LSN). Phantom = ranges no read
set. Resolver single-thread **280k TPS**
microbench. Falsos positivos se um TX aborta
depois de um subset de Resolvers actualizar
`lastCommit` — limitados à janela MVCC (**5 s**).

Logging (Fig. 2): mutação tagged para as SS
equipas; vazia nos outros LogServers. Lag SS←LS
produção 12 h: p999 médio **3.96 ms**, máx
**208.6 ms** (Fig. 3). Recovery (Fig. 4): sem
ARIES redo/undo no caminho crítico. RV =
min(DV); PEV = max(KCV). Primeira TX da epoch
diz às SS para descartar memória > RV. “Cheap
because redo is the normal log-forward path.”

**§2.5 Replicação (p. 6–7).** Metadata: quorum
Paxos nos Coordinators. Logs e storage:
**k = f+1** (não 2f+1) — falha de LogServer
*dispara recovery do TS*, não é mascarada.
Storage *teams* hierárquicos (host + processo)
> Copyset [29].

**§2.6 (p. 7).** Batching dinâmico no Proxy.
Atómicos (add, CAS-clear, set-versionstamp) —
Record Layer usa-os para índices agregados.

**§3 Geo (p. 7–8).** WAN: writes síncronos só no
DC primário + satélites da *mesma* região
(evita RTT WAN); async para a região standby.
Failover automático; satélite 6.5 ms vs WAN
60.6 ms no cluster Apple (§5.1). Commit médio
**22 ms** < WAN precisamente por isso.

**§4 Simulation (p. 8–9).** Tudo determinístico;
sem threads (1 processo FDB / core). Flow =
C++ async/await + actors. Simulador discreto:
rede, disco, tempo, RNG. Oracles: asserts nas
workloads, recoverability. Faults: máquina /
rack / DC, partições, latência, corrupção de
writes não-sync no reboot. **Buggification**:
erro extra, delay, parâmetro invulgar.
**Swarm testing** [40] (cluster size, workloads,
faults, subset de buggify). `TEST(cond)` para
cobertura condicional. Discrete-event corre
mais rápido que wall-clock em baixa
utilização. Limitações: não apanha
performance/load-balance; não testa libs
terceiros / OS / FS — por isso reescreveram
Paxos e evitaram dependências. “bugs in
critical dependent systems … can lead to bugs
in FDB.”

**§5 Eval (p. 9–11).** Produção geo 58 máquinas,
292 TB, 862 processos. Mês: 390k reads/s,
138k writes/s, 1.47 M keys/s. Read 1 / 19 ms
(avg / p999); commit 22 / 281 ms. Uma recovery
em Ago 2020: **8.61 s** → “five 9s”. Lab 4–24
máquinas: write T100 67→391 MBps (5.84×);
90/10 593k→2.78 M ops/s. GRV ~1 ms, read
0.35 ms, commit 2 ms abaixo de 100k ops; em
saturação commit **368 ms**. 289 recoveries
produção: mediana **3.08 s**, p90 **5.28 s**.
Reads do cliente **não** bloqueiam (SS
continuam).

**§6 Lessons (p. 11).** Unbundling permite
colocar papéis em instance types diferentes e
trocar o engine (RocksDB “ongoing”). Simulation
= produtividade: bugs em produção primeiro
reproduzidos no sim. CloudKit: **> 0.5 M disk
years sem uma corrupção**. Upgrade = restart
de todos os processos em segundos (protocolo
RPC não precisa de dual-version). Fast recovery
*escondeu* bugs no DataDistributor (reinício
com o Sequencer “curava” estado). Janela 5 s:
a maioria dos TXs longos são apps ineficientes;
backup parte o scan em ranges.

**§7–8.** Percolator/Tephra/Omid = SI em cima de
KV; FDB = strict serializability. Calvin/H-Store
= timestamp order. Aurora = log no storage.
Testing: Jepsen não é determinístico; model
check não é o código.

## Tese central e argumento

Duas teses, não uma:

1. **Produto:** KV ordered + TX strict-serializable
   + *nada mais* — SQL/índices/documentos são
   layers (p. 2). É a identidade que RFC-0017
   chama “FDB-shaped”.
2. **Engenharia:** simulação determinística do
   *binário real* + “failure = recovery” (um
   caminho) + unbundling TS/LS/SS. O f+1 e o
   5 s MVCC são *consequências* dessa
   arquitectura, não leis a copiar para um
   multi-Raft.

Montanha já escolheu (1). RFC-0017 recusou
clonar (2) no dia um. Esta ficha **não reabre**
esse recusar: o paper descreve um sistema com
Sequencer singleton, janela 5 s, e recovery que
aborta o *mundo* de writes por ~3 s. Pedra+Raft
por range é outra geometria.

## Estrutura do texto

| § | p. | Conteúdo |
|---|---:|----------|
| Abstract / 1 | 1 | layers, sim, NewSQL |
| 2.1–2.3 | 2 | princípios, API, Fig. 1 |
| 2.4 TX | 4 | OCC+MVCC, Alg. 1, log, recovery |
| 2.5–2.6 | 6 | f+1, batching, atómicos |
| 3 Geo | 7 | satélites, WAN async |
| 4 Simulation | 8 | Flow, buggify, swarm |
| 5 Eval | 9 | 22 ms commit, 3 s recovery |
| 6 Lessons | 11 | 0.5 M disk-years, 5 s window |
| 7–8 | 12 | related, conclusão |

## Conceitos-chave

- **Unbundled [50].** TS / LS / SS escalam à
  parte. Termo de Lomet et al.; FDB decompõe
  ainda o TS em papéis.
- **Sequencer.** Uma fonte de versões; 1 M/s.
- **Resolver.** OCC range-partitioned; lock-free
  (Alg. 1).
- **Make failure common.** Desligar o TS e
  recuperar, em vez de mascarar com 2f+1.
- **f+1.** Logs/storage; falha ⇒ recovery, não
  quorum.
- **5 s MVCC.** Limite de memória e de duração
  de TX (p. 11).
- **Buggification / swarm.** Fault injection
  *dentro* do contrato, não só da rede.
- **Layer.** Processo stateless sobre o KV.

## Citações relevantes

1. "FoundationDB adopts an unbundled architecture that decouples an in-memory transaction management system, a distributed storage system, and a built-in distributed configuration system." (p. 1)
2. "before building the database itself, we built a deterministic database simulation framework that can simulate a network of interacting processes and a variety of disk, process, network, and request-level failures and recoveries, all within a single physical process" (p. 2)
3. "FDB does not rely on quorums to mask failures, but rather tries to eagerly detect and recover from them by reconfiguring the system. This allows us to achieve the same level of fault tolerance with significantly fewer resources: FDB can tolerate f failures with only f+1 (rather than 2f+1) replicas." (p. 2)
4. "we handle all failures through the recovery path: instead of fixing all possible failure scenarios, the transaction system proactively shuts down when it detects a failure" (p. 3)
5. "Transaction size is limited to 10 MB" (p. 3)
6. "Because Tx observes the results of all previous committed transactions, FDB achieves strict serializability." (p. 4)
7. "the transaction conflict rate is very low (less than 1%) and OCC works well" (p. 5)
8. "the 99.9 percentile of the average and maximum delay is 3.96 ms and 208.6 ms, respectively" (p. 5, Fig. 3, lag SS←LS)
9. "FDB chooses a 5-second MVCC window to limit the memory usage of the transaction system and storage servers" (p. 11)
10. "CloudKit has deployed FDB for more than 0.5M disk years without a single data corruption event" (p. 11)
11. "early versions of FDB depended on Apache Zookeeper for coordination, which was deleted after real-world fault injection found two independent bugs in Zookeeper (circa 2010) and was replaced by a de novo Paxos implementation written in Flow" (p. 11)
12. "The median and 90-percentile [reconfiguration] are 3.08 and 5.28 seconds, respectively" (p. 11, Fig. 10, n=289)
13. "Simulation is not able to reliably detect performance issues … It is also unable to test third-party libraries or dependencies" (p. 9)
14. "For commits, the average and 99.9-percentile are about 22 and 281 ms, respectively" (p. 10; WAN 60.6 ms, satélite 6.5 ms)
15. "the debugging process was almost always first to improve the capabilities or the fidelity of the simulation until the issue could be reproduced there" (p. 11)

## Diálogo teórico

- **Deuteronomy [48, 51]** — unbundled; MVCC no
  TS. FDB põe MVCC no SS (p. 3).
- **Active Disk Paxos [27]** — Coordinators.
- **Kung–Robinson OCC [44], MVCC [18]** — SSI
  (p. 2, 4).
- **ARIES [53]** — recovery que *não* usam.
- **Percolator [55] / R041** — SI em cima de KV;
  FDB é strict serializable + Sequencer.
- **Calvin [64]** — ordem por timestamp; related.
- **Spanner [31] / CRDB [62]** — locks + relógio.
- **Aurora [66]** — redo no storage.
- **Record Layer [28] / R044** — layer canónica.
- **Jepsen [3]** — sem reprodutibilidade
  determinística (p. 12).
- **Yuan deep bugs [46]** — o que o sim apanha.

## Relação com Pedra / Montanha

RFC-0017 já trava o clone do split de papéis e
pede simulação de *cluster*. Esta ficha dá locus.

| FDB (paper) | Montanha / Pedra | Ledger |
|-------------|------------------|--------|
| KV + TX; SQL é layer | L9, DCS/SQL/fold | **L9 `SHIP` src+=ficha R043** |
| Simulação do binário real + buggify | DST kernel + `cluster_dst_*` + FailingEnv; **não** Flow/swarm do cluster inteiro | **L28 `MEASURE`**: aprofundar swarm/buggify no store (já é a cultura; o paper é o teto, não o P0) |
| Sequencer + Proxy + Resolver | multi-Raft + 2PC de intents + SnapshotTx | **L29 `REFUSE`** clonar o TS unbundled no dia um (já no RFC-0017 §out-of-scope). 5 s de unavailability de writes e Sequencer singleton não cabem no “majority por range” |
| f+1 + recover | Raft 2f+1 por range | **não** mudar. f+1 do paper *é* o recovery de 3 s |
| 5 s MVCC / TX 10 MB | Pedra sequence sem janela wall-clock | **L30 `REFUSE`** no kernel. Se um dia houver TX FDB-class no store, a janela é um *produto*, não um default silencioso |
| OCC+MVCC strict serializable | OccTransaction local (read+write set); store 2PC write-write | não reabre L22. FDB não é Percolator |
| Storage = SQLite B-tree | Pedra LSM | confirma L9: o engine é plugável; eles próprios falam em Rocks “ongoing” (§6.1) |
| WAL = LogServers | Pedra WAL + Raft log | L25 intacto (MEASURE à parte) |

- **Não implementar** Sequencer/Resolver/LogServer.
- **DST:** o paper justifica *mais fidelidade* no
  sim (clock, unsynced write corruption, swarm),
  não um rewrite em Flow. Gate L28: um bug de
  cluster reproduzido primeiro no sim, com seed,
  sem “não conseguimos reproduzir”.
- **Falsificaria L29:** um bench em que o 2PC
  cross-range do store perde >2× para um
  Sequencer-class *e* o DST de recovery de 3 s
  for aceitável para Caixote. O paper não mede
  isso no nosso hardware.

Fichas irmãs: R041 Percolator (D4); R044 Record
Layer (próxima); R010 (library local); R048
(learner; outra geometria Raft).

## Avaliação crítica

- **Produção Apple geo, SSD, multi-tenant
  CloudKit.** Conflito 0.73% e 22 ms commit
  assumem essa carga. Caixote/DCS pode ser mais
  contended (leases).
- **5 nines** = 1 recovery de 8.61 s num mês
  *daquele* cluster, não um SLA formal.
- **0.5 M disk-years sem corrupção** é o melhor
  número do paper — e depende do sim + do facto
  de *não* terem ZooKeeper. Não prova o
  multi-Raft Pedra.
- **Lab 24 máquinas / 10 GbE** (Fig. 8) é
  saturado em CPU de Proxy/Resolver/Log — o
  bottleneck *é* o unbundling.
- **Simulation não testa FS/OS** (p. 9) — Pedra
  `FailingEnv` cobre uma fatia; o resto é o
  mesmo buraco.
- **Não medem:** LSM vs SQLite no SS; DST vs
  Jepsen head-to-head; TX > 5 s além de
  “parta o job”.

## Palavras-chave

foundationdb; unbundled; occ; mvcc; sequencer;
resolver; simulation; buggify; swarm; layers;
strict-serializability; recovery; f-plus-one

## Fontes

**Primária:** `research/fontes/R043_Zhou_2021_FoundationDB.pdf`
(SIGMOD ’21, PDF foundationdb.org) + `.txt` +
`.pages.txt`.

**Secundárias (não citadas como se fossem este
paper):** RFC-0017; RFC-0021/0022;
`docs/montanha-vs-foundationdb.md`; ficha R041.
Record Layer (R044) **não** foi lido nesta ficha.
