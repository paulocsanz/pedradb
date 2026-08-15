# Fichamento: R041 — Percolator (TX + observers on KV)

**Status:** D4
**Lido em:** 2026-08-14
**Catalog:** [CATALOG.md](../CATALOG.md) · `R041`

## Referência

PENG, Daniel; DABEK, Frank. **Large-scale Incremental Processing
Using Distributed Transactions and Notifications.** In: *9th USENIX
Symposium on Operating Systems Design and Implementation
(OSDI ’10)*, Vancouver, BC, 4–6 Oct. 2010. p. 1–14 (PDF interno).
URL: https://www.usenix.org/legacy/event/osdi10/tech/full_papers/Peng.pdf

Locus abaixo = **p. interna do PDF** (1–14). Figs. 4–6 (protocolo)
e 7–10 (eval) conferidas no texto extraído; Fig. 4 é diagrama de
células, descrita pelo paper.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract, §§1–5,
  Figs. 1–10, Figs. 2/5/6 (API + colunas + pseudocódigo), §3
  eval (Caffeine, microbench, TPC-E-like), §4 related, §5
  conclusion. Referências [1]–[35] **não** foram relidas como
  obras; Berenson SI [5] e Bernstein [6] só pelo que o texto
  afirma.
- **Arquivo lido:** `docs/references/percolator-osdi2010.pdf` +
  extração paginada `docs/references/percolator-osdi2010.pages.txt`.
- Como foi lido: PDF local, seção a seção, nesta sessão.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 1–2).** Manter o índice web exige mutações
pequenas e independentes sobre dezenas de PB, milhares de
máquinas, milhares de milhões de updates/dia. DBMS clássico
não escala; MapReduce reprocessa o repositório inteiro (latência
∝ tamanho do corpus, não do update); Bigtable escala mas
não dá invariantes sob updates concorrentes. Percolator =
acesso aleatório + **transações ACID (snapshot isolation)**
+ **observers** (código disparado quando uma coluna muda).
Produção: índice live do Google. Mesmo crawl/dia que o
pipeline em batch; **idade média** dos documentos nos
resultados **−50%**. Mediana de processamento **~100×**
mais rápida (p. 2). Não substitui MapReduce (sort global),
nem Bigtable sem consistência, nem um DBMS pequeno.

Arquitetura (Fig. 1, p. 2): worker Percolator + tablet
Bigtable + chunkserver GFS em cada máquina; **timestamp
oracle** (timestamps estritamente crescentes) e lock service
leve (só para o scan de notiﬁcações). Sem gestor central de
TX; sem detector global de deadlock. Latência de cleanup de
locks preguiçoso “tens of seconds” — aceitável para índice,
não para OLTP (p. 2).

**§2.1 Bigtable (p. 2–3).** Mapa (row, column, timestamp).
Row transaction = RMW atómico **numa** row. Percolator
mantém a forma da API e mete metadata em colunas
especiais (Fig. 5). O que Bigtable **não** dá: TX multi-row
e o framework de observers.

**§2.2 Transactions (p. 3–6).** SI [5]: reads no *start
timestamp*, writes num *commit timestamp* posterior
(Fig. 3). Protege write-write; **não** é serializável;
**write skew** permitido (p. 3). Vantagem: Get não pega
lock — lookup Bigtable no timestamp. Cliente coordena
2PC; interacção entre máquinas = row transactions no
tablet. Locks **persistem** no Bigtable (colunas in-memory
do mesmo tablet): se um lock desaparecesse entre as duas
fases, duas TXs que deviam conﬂituar poderiam ambas
commitar (p. 3–4).

Protocolo (Figs. 4 e 6, p. 4–5) — cada coluna `c` vira
3 colunas Bigtable + notify/ack:

| Coluna | Papel |
|--------|--------|
| `c:data` | payload no *start ts* |
| `c:lock` | TX uncommitted; aponta para o **primary lock** |
| `c:write` | commit visível; aponta para o ts dos dados |
| `c:notify` | hint de observer (não transaccional) |
| `c:ack O` | último start ts em que o observer O correu |

`Set` faz buffer. **Prewrite:** para cada célula, abort se
há `write` depois do start ts (linha 32) **ou** lock em
qualquer ts (linha 34); senão escreve data+lock no start
ts. Uma célula é o *primary* (arbitrário: `writes[0]`).
**Commit:** pede commit ts ao oracle; substitui o lock
primary por `write` (commit point, linha 58); depois
secondaries (sem row-txn obrigatório). Get: se há lock
em `[0, start]` espera / tenta cleanup; senão lê o último
`write` ≤ start e segue o ponteiro para `data`.

Crash do cliente: locks órfãos. Cleanup **lazy** no Get
do próximo. Primary é o ponto de sincronização: só
commit **ou** cleanup modifica o primary sob row-txn.
Se o primary já é `write`, roll-forward dos secondaries;
senão rollback. Liveness: token Chubby + wall-time no
lock (p. 6). Worker vivo mas stuck também é limpo.

**§2.3 Timestamps (p. 6).** Oracle aloca um range em
disco e serve o resto da RAM; restart salta para a frente,
nunca para trás. **~2 milhões de timestamps/s** numa
máquina. Cada TX contacta o oracle **duas vezes**
(start + commit); workers batelam. Prova de visibilidade
(p. 6): se \(T_W < T_R\), W pediu \(T_W\) antes de R
receber \(T_R\); W escreveu locks **antes** de pedir
\(T_W\); logo R ou vê o `write` ou o lock.

**§2.4 Notifications (p. 6–8).** Observer ≠ trigger:
corre noutra TX; write disparador e writes do observer
**não são atómicos** (p. 6). ~10 observers no índice.
“At most one observer commits per change”; o conversos
não (message collapsing). Implementação: coluna
`notify` (locality group próprio) + `ack`; scan
distribuído aleatório por tablet; lock advisory para não
dois workers na mesma row; “teleport” quando dois
scanners se encontram (**bus clumping**, p. 7). Weak
notify: só a coluna Bigtable `notify`, sem TX — para
hotspots (muitos dups a disparar o mesmo cluster).

**§2.5 Discussion (p. 8).** ~50 ops Bigtable por
documento vs 1 read GFS no MapReduce. Conditional
mutations (1 RPC em vez de 2) + batch de locks (atraso
de segundos) + prefetch de colunas na mesma row
(**10×** menos reads). Thread-per-request, milhares de
threads; kernel patches internos.

**§3 Evaluation (p. 8–12).** Cluster Google, Linux,
x86, SATA commodity.

*Caffeine vs MapReduce (p. 8–10).* Pipeline antigo:
~100 MRs, 2–3 dias por documento. Caffeine: 10
observers; mediana **>100×**; índice **3×** maior;
**~2×** os recursos para o mesmo crawl. Fig. 7 (240
máquinas, 1B docs sintéticos, 3 clustering keys): a
1%/h de crawl, MR ~30 min de atraso médio, Percolator
**~2 s** (~1000×). Saturam a 40%/h (fila unbounded).
MR ganha em crawl rate alto (stream ≫ random I/O).
Regime real = single-digit %/h.

*Microbench (Fig. 8, p. 10).* 1 tablet, cache quente,
batching off. Read/s 15513 → 14590 (**0.94×**).
Write/s 31003 → 7232 (**0.23×**, ~4× overhead):
read de locks + write lock + write unlock. Pior caso =
TX de 1 célula.

*TPC-E-like (p. 11–12).* Latência média **2–5 s**,
outliers **minutos**. Omitiram o update do broker
(hotspot 1/5 s). Fig. 9: 11 → 15 000 cores, TPS
linear até **11 200 tps**. Referência comercial 3 183
tpsE / 64 Nehalem; estimam **~30× mais CPU** por
TX. Fig. 10: matar 1/3 dos tablets — dip e recover.

**§4 Related (p. 12–13).** MapReduce / DryadInc /
MR incremental = meio-termo. SI = MVTO [6] + 2PC.
Observers ≈ view maintenance mas o índice raw→indexed
não cabe. Sinfonia mini-transactions mais pobres.
CloudTPS: LTMs intermédios, foco em latência/website.
ElasTraS: sem observers. Dynamo/PNUTS/NoSQL: store
sem transformação. Percolator aceita **um** DC por
consistência mais estrita; replicação Bigtable
cross-DC é per-tablet e **parte invariantes** de uma
TX distribuída (p. 13).

**§5 (p. 13).** Em produção no websearch desde Abril
2010. Futuro: de onde vem o 30× de CPU.

## Tese central e argumento

Tese (p. 1–3): sobre um KV *dumb* (Bigtable row-txn
só), dá para construir SI multi-row se (i) versões
vivem na dimensão timestamp, (ii) locks e write-records
são colunas persistentes na **mesma** row, (iii) o
cliente faz 2PC com um *primary lock* como ponto de
commit, (iv) um oracle dá timestamps monotónicos.
Observers estruturam o compute incremental **fora**
da TX que dispara.

Não é um motor de storage. É uma **camada** (client
library) em cima de Bigtable/GFS. O paper admite o
preço: 4× no write pontual (Fig. 8), 30× CPU vs
DBMS (p. 11), latência de segundos.

## Estrutura do texto

| § | Início | Conteúdo |
|---|-------:|----------|
| Abstract | p. 1 | −50% idade; gap MR/DBMS |
| 1 Introduction | p. 1 | Caffeine, 100×, 3 usos |
| 2 Design | p. 2 | workers, oracle, SI |
| 2.1 Bigtable | p. 2 | row-txn, locality groups |
| 2.2 Transactions | p. 3 | Figs. 4–6, 2PC, cleanup |
| 2.3 Timestamps | p. 6 | 2M ts/s, prova de visibilidade |
| 2.4 Notifications | p. 6 | observers, clumping, weak notify |
| 2.5 Discussion | p. 8 | RPCs, batch, prefetch, threads |
| 3 Evaluation | p. 8 | Caffeine, Fig. 7–10 |
| 4 Related | p. 12 | SI, Sinfonia, CloudTPS |
| 5 Conclusion | p. 13 | prod. Abril 2010 |
| References | p. 14 | [1]–[35] |

## Conceitos-chave

- **Snapshot isolation (não serializável).** Write-write
  abort; write skew permitido (p. 3, [5]). Termo de
  Berenson; Percolator implementa.
- **Primary lock.** Célula de sincronização commit vs
  cleanup (p. 5). Sem isto o lazy cleanup é race.
- **Prewrite / write-record.** Data no start ts; visível
  só quando `c:write` no commit ts aponta para ela
  (Fig. 4).
- **Timestamp oracle.** Serviço pequeno, ranges em
  disco, 2 RPCs/TX (p. 6).
- **Observer ≠ trigger.** Outra TX; não mantém
  invariante atómica (p. 6).
- **Message collapsing.** Vários writes → um observer
  (p. 7).
- **Weak notification.** `notify` sem TX, para hotspot
  (p. 7–8).

## Citações relevantes

1. "By replacing a batch-based indexing system with an indexing system based on incremental processing using Percolator, we process the same number of documents per day, while reducing the average age of documents in Google search results by 50%" (p. 1, abstract)
2. "we currently implement snapshot isolation semantics" (p. 2)
3. "Snapshot isolation does not provide serializability; in particular, transactions running under snapshot isolation are subject to write skew" (p. 3)
4. "Percolator has no central location for transaction management; in particular, it lacks a global deadlock detector." (p. 2)
5. "if a lock could disappear between the two phases of commit, the system could mistakenly commit two transactions that should have conflicted" (p. 3–4)
6. "The basic approach for committing buffered writes is two-phase commit, which is coordinated by the client." (p. 4)
7. "if the transaction sees another write record after its start timestamp, it aborts (line 32); this is the write-write conflict that snapshot isolation guards against. If the transaction sees another lock at any timestamp, it also aborts (line 34)." (p. 4)
8. "Once the primary’s write is visible (line 58), the transaction must commit since it has made a write visible to readers." (p. 5)
9. "Our oracle serves around 2 million timestamps per second from a single machine." (p. 6)
10. "unlike database triggers, they cannot be used to maintain database invariants. In particular, the triggered observer runs in a separate transaction from the triggering write" (p. 6)
11. "Percolator performs around 50 individual Bigtable operations to process a single document." (p. 8)
12. "When doing a write, Percolator incurs roughly a factor of four overhead on this benchmark." (p. 10; Fig. 8: Write/s 31003 → 7232)
13. "Our average transaction latency is 2 to 5 seconds, but outliers can take several minutes." (p. 11)
14. "we estimate that Percolator uses roughly 30 times more CPU per transaction than the benchmark system." (p. 11)
15. "the median document moves through Caffeine over 100x faster than the previous system" (p. 9)
16. "Percolator implements snapshot isolation by extending multi-version timestamp ordering across a distributed system using two-phase commit." (p. 12)
17. "replication is likely to break invariants between writes in a distributed transaction" (p. 13; replicação Bigtable cross-DC é per-tablet)

## Diálogo teórico

No texto (não a biblio inteira):

- **Bigtable [9]** — o KV; row-txn é o único atómico
  nativo.
- **MapReduce [13]** — o sistema que substituem; ainda
  melhor quando o update ≈ tamanho do corpus (Fig. 7).
- **Berenson SI [5]** — semântica + write skew.
- **Bernstein/Goodman [6]** — MVTO; o paper diz que
  estende isto com 2PC (p. 12).
- **GFS [20] / Chubby [8]** — persistência e liveness.
- **Sinfonia [2,3]** — mini-txn + notify antigo; mais
  pobre semanticamente.
- **CloudTPS [34] / ElasTraS [12]** — ACID sobre
  HBase/Bigtable com camada de servidores; Percolator
  é *client-coordinated*.
- **Dynamo [14] / PNUTS [11]** — store sem
  transformação; Percolator escolhe um DC.
- **TPC-E [1]** — admitem que é o benchmark errado
  (OLTP vs incremental).
- **Stonebraker/DeWitt [16]** — crítica ao MR sem
  índices; Percolator *é* esses índices.

## Relação com Pedra

Três sítios distintos — não misturar:

| Camada | O que há hoje | Vs Percolator |
|--------|----------------|---------------|
| **Kernel** `OccTransaction` | Snapshot = `last_sequence()` no begin; commit valida **read-set ∪ write-set** (`key_has_write_after`); um WAL record sob write lock (`occ.rs`) | SI *mais* write-set. **Mais forte** que Percolator: um write concorrente numa key que só *leste* aborta (corta write skew nessa key). Sem colunas `lock`/`write`, sem prewrite, sem oracle RPC. Latência = fsync local, não 2–5 s. |
| **Montanha-Store** | Mesmo range: `put_batch` = um log Raft + `apply_batch`. Cross-range: **2PC de intents duráveis** (`tx_start` / `tx_finish`, FDB-class, `lib.rs`). `SnapshotTx` = geração no begin + OCC no commit. `WatchHub` pós-majority. | 2PC **não** é Fig. 4/6. Intents por range, não primary lock numa célula user. Sem timestamp oracle. Watch ≠ observer (não corre código na store). |
| **Fold / CHANGELOG** | L8: cursor após apply (`LocalApplied`) | Incremental *consume* o log; não é observer-sobre-coluna. |

- **L22 (novo) `REFUSE` — protocolo Percolator (locks em
  colunas + 2PC cliente) no kernel e como 2PC do
  store.** O kernel já tem OCC+WAL. O store já tem
  intents FDB. Copiar Fig. 6 meteria 4× no put
  pontual (Fig. 8) e cleanup de dezenas de segundos
  (p. 2) no caminho que o posicionamento pede
  sub-ms. Teste que *não* vamos escrever: `c:lock`
  no SST.
- **L23 (novo) `REFUSE` — timestamp oracle como
  serviço.** No kernel o sequence da LSM *é* o
  relógio. No store, `snapshot_begin` já pinta uma
  geração. Um oracle estilo p. 6 (2 RPCs/TX, batch,
  2M ts/s) só faria sentido se adoptássemos SI
  Percolator cross-range — o que L22 recusa.
- **Observers:** não são L8 e não substituem Watch.
  Semântica “outra TX, collapsing, weak notify”
  pode um dia ser uma *layer* de indexação
  incremental. **Não** entra no kernel. Sem linha
  nova no ledger até haver produto que peça
  “Caffeine local”.
- **O que o paper confirma e o Pedra já faz:**
  (1) TX multi-key sobre um KV que só é atómico
  por row/range — L9 (produtos são camadas);
  (2) locks/intents têm de ser **duráveis** entre as
  duas fases (p. 3–4) — o store já persiste
  prepare; (3) SI ≠ serializable — o OCC do kernel
  é *mais* estrito no read-set, e isso é
  desejável (índice secundário + row no mesmo
  commit, `positioning.md`).
- **Não reabre L1–L10.** WiscKey/vlog não aparecem.
  Compact/LL não aparecem.
- **Falsificaria um port do protocolo:** (1) o
  `baseline` de commit local ficar 4× mais lento
  (Fig. 8); (2) um DST em que o primary commitou e
  um secondary ficou lock — o paper resolve com
  roll-forward no Get; o Pedra resolve com Raft
  apply + revert de preimage (`txn_kernel.rs` F34).
  São stories diferentes; misturá-las é silent-wrong.

Fichas irmãs: R048 TiDB (D4); R043 FDB (D4 — strict
serializable + Sequencer, não Percolator). Ainda
inexistentes: R044 Record Layer, R053 Calvin, R086
Kung–Robinson OCC.

## Avaliação crítica

- **Workload = índice web, não OLTP.** Eles dizem-no
  três vezes (p. 2, 11, 13). Os 100× / −50% / 2 s
  são vs MapReduce de corpus, não vs Rocks/Pebble
  embed.
- **Fig. 8 é 1 tablet, cache quente, batch off,
  TX de 1 célula.** O 0.23× write é o tecto do
  protocolo, não o Caffeine em produção (lá há
  batch de segundos).
- **TPC-E “like”:** omitiram o hotspot do broker;
  não cumprem latência TPC-E; 30× CPU é
  estimativa contra uma máquina Nehalem 64-core
  (p. 11). Não citar como “Percolator é 30× pior
  que Pedra”.
- **Oracle single machine 2M ts/s** — SPOF
  mitigado por ranges em disco; restart salta ts.
  Não é o relógio do Pedra.
- **Cross-DC:** eles *recusam* replicação Bigtable
  para TX (p. 13). Montanha majority-in-range é
  outra geometria.
- **Não medem:** serializability, write-skew
  injectado, fail-stop do oracle a meio do
  commit, comparação com FDB/TiKV (2010).

## Palavras-chave

percolator; snapshot-isolation; two-phase-commit;
timestamp-oracle; bigtable; observers; incremental;
caffeine; write-skew; primary-lock; occ

## Fontes

**Primária:** `docs/references/percolator-osdi2010.pdf`
(OSDI ’10, USENIX, 14 pp.) +
`docs/references/percolator-osdi2010.pages.txt`.

**Secundárias (não citadas como se fossem este paper):**
`crates/pedradb-core/src/occ.rs`;
`crates/pedradb-store/src/lib.rs` (`tx_start` /
`commit_tx`); RFC-0022 SnapshotTx; `docs/positioning.md`.
FDB / TiKV / Calvin **não** foram lidos nesta ficha.
