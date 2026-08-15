# Fichamento: R048 — TiDB (Raft learner ≠ voter)

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R048`

## Referência

HUANG, Dongxu; LIU, Qi; CUI, Qiu; FANG, Zhuhe (corr.); et al.
**TiDB: A Raft-based HTAP Database.** *PVLDB* 13(12):3072–3084,
2020 (VLDB ’20). DOI 10.14778/3415478.3415535.
URL: https://www.vldb.org/pvldb/vol13/p3072-huang.pdf

Páginas internas do PDF: 1–13 (= VLDB 3072–3084). Locus =
**p. interna**. Figs. 1, 7, 10–12 e Tables 2–4 conferidas no
texto extraído.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract, §§1–8,
  Figs. 1–12, Tables 1–4, Eqs. 1–5. Referências [1]–[38] **não**
  foram relidas como obras; Percolator [33] e Raft [29] só pelo
  que o texto afirma (R041 já fichada).
- **Arquivo lido:** `docs/references/tidb-raft-htap-vldb2020.pdf`
  + extração `docs/references/tidb-raft-htap-vldb2020.pages.txt`.
- Como foi lido: PDF local, seção a seção, nesta sessão.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**§1 Introduction (p. 1–2).** HTAP precisa de *freshness*
(análise do dado recente) e *isolation* (OLTP e OLAP não se
comerem). ETL leva horas; in-memory no mesmo servidor
(HyPer −5×, HANA −3× no CH-benCHmark [34]) não isola e
não escala. Tese: estender um RSM (Raft/Paxos) com réplicas
**dedicadas** a OLAP. TiDB = multi-Raft row store (TiKV) +
learners que materializam coluna (TiFlash) + SQL engine +
TiSpark. “To the best of our knowledge, this idea has not
been studied before” (p. 2) — como *paper de sistema*, não
como teorema.

**§2 Raft-based HTAP (p. 2).** Fig. 1: grupo = leader +
followers (quorum) + **learner**. O learner:
- não entra em eleição;
- **não** conta para o quorum;
- replica o log **em assíncrono** (o leader responde ao
  cliente sem esperar);
- consistência forte **no read** (não no write);
- transforma row → column.

Porque não basta mais followers: “each follower can become
the leader” → não isola recursos; e “the leader must wait
for responses from a larger quorum” → piora o write (p. 2).

**§3 Architecture (p. 3).** Fig. 2: client → SQL / TiSpark
→ TiKV (row) + TiFlash (column); PD = scheduler +
**timestamp oracle**. KV: `Key:{table{id} record{rowID}}`.
Range partition = **Region** (~96 MB default, §4.1).
TX = 2PC **Percolator** [33]. TiKV e TiFlash em
hardware separado.

**§4.1 TiKV (p. 3–5).** RocksDB por nó. Raft sequencial
é o bottleneck; optimizam: append local ∥ send; batch;
apply noutro thread; *read index* e *lease read* (Raft
[29]); *follower read* (pede read index ao leader). PD
balanceia, split/merge de Regions (split = log de metadata
no grupo; merge = 2PC local entre dois grupos). Heartbeat
só em Regions ocupadas.

**§4.2 TiFlash (p. 5–6).** `ALTER TABLE x SET TiFLASH
REPLICA n`. Init = snapshot se o log for grande. Replay
FIFO (Table 1): compact rollbacked prewrites → decode
tuples → transform colunas → DeltaTree. Schema cache +
syncer (regular / compulsive se mismatch). DeltaTree
(Fig. 4): deltas append-only (também WAL) + stable
chunks estilo Parquet (LZ4, metadata em ficheiros
separados) + B+ tree nos deltas. Table 2: count() ~2×
mais rápido que LSM (universal compaction) em 100–200 M
tuples; WA DeltaTree **16.11** vs LSM **4.74**.

**§4.2.4 Read (p. 6).** “Like follower read, learner nodes
provide snapshot isolation.” O learner **manda read index
aos leaders**; os leaders enviam os logs em falta; só
depois lê o DeltaTree. Isto é o contrário de “nunca falar
com o SoR”.

**§5 Engines (p. 6–8).** SI ou RR; 2PC Percolator
optimista (locks no prewrite) ou pessimista (locks nas
DMLs + `for_update_ts`). PD ts: físico (ms) + 18 bits
lógicos; ~1 M ts/s; TPC-C pede ≤ 6000 ts/s/servidor
(§6.2). Optimizer: RBO+CBO; três caminhos de scan
(row / index / column), Eqs. 1–5; coprocessor +
vectorização. TiSpark: snapshot ts do PD, pushdown,
2PC para load. Isolamento: engines e stores em
servidores distintos; AP em TiKV limitado a **500 MB**
por default; réplica extra de coluna para checks de
unicidade do TP.

**§6 Experiments (p. 9–11).** 6× (2× E5-2630 v4, 188
GB, 10 GbE), CentOS 7.6. CH-benCHmark (TPC-C + 22
TPC-H adaptadas). 100 W ≈ 70 GB.

- **OLTP (Fig. 7):** TiDB ≥ CRDB na maioria dos
  pontos; optimista ganha excepto 50–100 W + 1024
  clientes. Latência em **µs** no gráfico (d) a 200 W.
- **PD ts (Table 3):** 6 servidores × 602 594 ts/s;
  quase nenhum ≥ 2 ms.
- **OLAP (Fig. 8):** TiKV+TiFlash **sempre** melhor
  que um store só. Q8: 1.13 s (KV) / 1.64 s (Flash) /
  **0.55 s** (ambos). Fig. 9: TiSpark ≈ SparkSQL em
  Parquet; Presto/Greenplum é outra engine.
- **HTAP (Fig. 10):** 3 TiKV + 3 TiFlash. Com o
  mesmo nº de TP clients, mais AP clients **cortam
  ≤ 10%** do TPS. AP cai ≤ 5% com mais TP. Contraste
  MemSQL (Fig. 12): TPS **>5×** pior com AP.
- **Lag (Fig. 11, Table 4):** 10 W: ~99% < 500 ms.
  100 W + 32 AC: **84–87% < 1 s**; só ~49% < 100 ms.
  “freshness of about one second” (p. 11). Lag cresce
  com o tamanho dos dados e com a *frequência* de
  reads no learner (cada read puxa read-index).

**§7 Related (p. 11–12).** Oracle IM, SQL Server
Apollo+Hekaton, HANA async copy (sub-second, TP
single-node). Wildfire (Parquet, last-write-wins,
sem consenso). SnappyData, MemSQL (não isola),
HyPer/ScyPer, BatchDB (row replica, sem HA),
L-Store, Peloton (single-node). **CRDB [38]:**
mesmo Raft+TX, **serializable**, “does not support
dedicated OLAP or HTAP” (p. 12).

**§8 (p. 12).** “generic solution to evolve NewSQL
into HTAP”: learner colunar + hardware separado.

## Tese central e argumento

Tese (p. 2): isolation HTAP = **não** meter OLAP no
voter set. Learner async + transform + (no TiFlash)
read-index no leader. Isolation de *performance* vem
do hardware separado, não do Raft. Freshness ≈ lag
do log + custo do replay, medido em **centenas de
ms a ~1 s**, não em µs.

O paper mistura três coisas que o Pedra tem de
manter separadas:

1. **Papel Raft** (learner ≠ quorum ≠ eleição).
2. **Formato** (row → column no replay).
3. **Semântica de read** (read-index → SI).

RFC-0024 só precisa de (1). (2) é L7. (3) é o
oposto do fold (`get` LocalApplied, nunca
`get_strong` no request path).

## Estrutura do texto

| § | Início | Conteúdo |
|---|-------:|----------|
| Abstract | p. 1 | learner colunar, CH-benCHmark |
| 1 Introduction | p. 1 | freshness + isolation |
| 2 Raft-based HTAP | p. 2 | Fig. 1, porque não mais followers |
| 3 Architecture | p. 3 | TiKV / TiFlash / PD / 2PC |
| 4.1 TiKV | p. 3 | Raft opts, Region, read index |
| 4.2 TiFlash | p. 5 | replay, DeltaTree, **read index** |
| 5 Engines | p. 6 | Percolator 2PC, CBO, TiSpark |
| 6 Experiments | p. 9 | Figs. 7–12, Table 4 |
| 7 Related | p. 11 | CRDB sem HTAP |
| 8 Conclusion | p. 12 | NewSQL → HTAP |
| References | p. 13 | [1]–[38] |

## Conceitos-chave

- **Learner.** Replica o log; sem voto, sem eleição
  (p. 2). Termo Raft estendido pelo paper.
- **Quorum vs async.** Write espera majority de
  *voters*; learner é best-effort até o read.
- **Read index no learner.** SI exige ir ao leader
  (§4.2.4). Termo Raft [29]; aqui aplicado ao
  learner.
- **DeltaTree.** Delta + stable + B+ ; WA 16×
  (Table 2). Termo do paper.
- **Region.** Range ~96 MB, um Raft group.
- **Freshness.** Lag de replicação, não “latest
  committed no leader”.
- **Isolation (HTAP).** Hardware + store
  separado; *não* isolation SQL.

## Citações relevantes

1. "it asynchronously replicates Raft logs to learners which transform row format to column format for tuples, forming a real-time updatable column store" (p. 1, abstract)
2. "SAP HANA throughput was reduced by at least three times, and HyPer by at least five times" (p. 2; CH-benCHmark [34])
3. "Simply adding more followers, therefore, will not isolate resources. Moreover, adding more followers will impact the performance of the group because the leader must wait for responses from a larger quorum" (p. 2)
4. "A learner does not participate in leader elections, nor is it part of a quorum for log replication. Log replication from the leader to a learner is asynchronous; the leader does not need to wait for success before responding to the client." (p. 2)
5. "The strong consistency between the leader and the learner is enforced during the read time." (p. 2)
6. "TiDB implements a two-phase commit (2PC) protocol based on Percolator" (p. 3)
7. "They do not participate in the Raft protocols to commit logs or elect leaders so they induce little overhead on TiKV." (p. 5)
8. "After receiving a read request, the learner sends a read index request to its leaders to get the newest data that covers the requested timestamp." (p. 6, §4.2.4)
9. "Although the write amplification of DeltaTree (16.11) is greater than the LSM tree (4.74), it is also acceptable." (p. 6, Table 2)
10. "With the same number of TP clients, more analytical processing clients degrade the TP throughput at most 10% compared to no AP clients." (p. 11, Fig. 10)
11. "These metrics highlight that TiDB can guarantee data freshness of about one second on HTAP workloads." (p. 11, Table 4)
12. "as the number of AP clients increase, the transaction throughput significantly slows, dropping by more than five times" (p. 11–12, MemSQL Fig. 12)
13. "It offers a stronger isolation property: serializability, rather than snapshot isolation. However, it does not support dedicated OLAP or HTAP functionality." (p. 12, sobre CRDB)
14. "We also limit the default access table size on TiKV for analytical queries to at most 500 MB." (p. 8)
15. "PD is also our timestamp oracle, providing strictly increasing and globally unique timestamps." (p. 3)

## Diálogo teórico

No texto (não a biblio inteira):

- **Raft [29]** — RSM que estendem; read index /
  lease vêm daí.
- **Percolator [33] / R041** — 2PC do SQL engine
  (p. 3, §5.1). Confirma L22: o protocolo vive
  *acima* do store.
- **HyPer [18] / HANA [22] / MemSQL [3]** — o
  anti-padrão (mesmo servidor / sem isolation).
- **CH-benCHmark [13] + Psaroudakis [34]** — o
  número −3×/−5× da intro.
- **CRDB [38]** — primo NewSQL sem learner
  colunar.
- **Spanner [14]** — NewSQL de referência.
- **RocksDB [5]** — motor do TiKV (p. 3).
- **Stonebraker “one size” [37]** — o enquadramento
  HTAP (p. 1).
- **Wildfire / SnappyData / BatchDB / L-Store /
  Peloton** — HTAP sem o pacote Raft+learner.

## Relação com Pedra

| Peça TiDB | Pedra / Montanha | Acção |
|-----------|------------------|--------|
| Learner ≠ quorum ≠ eleição | RFC-0024: fold **não** é membro Raft | **L8 `SHIP` confirmado** (`src=ficha`) |
| Read-index no learner para SI | Fold = **LocalApplied**; request path **proibido** de `get_strong` | **L24 `REFUSE`** (novo) |
| Row→column no learner | L7 coluna no MANIFEST já `REFUSE` | não reabre L7; DeltaTree não entra no kernel |
| 2PC Percolator no SQL | L22 `REFUSE` no kernel; store já é intents FDB | confirma que Percolator é *layer* |
| PD timestamp oracle | L23 `REFUSE` | PD ~1 M ts/s não muda o kernel |
| Isolation = hardware separado | Fold numa máquina Caixote, SoR no cluster | alinhado; não é isolation SQL |

- **L8 fica `SHIP`, agora com ficha.** O paper é o
  locus industrial de “não metas o consumidor no
  quorum”. RFC-0024 já o dizia com Slipstream;
  R048 diz o mesmo a partir do Raft.
- **L24 (novo) `REFUSE` — read-index / SI no
  fold.** §4.2.4 torna o learner um *proxy* do
  leader no read. Isso **reintroduz** o SoR no
  request path — exactamente o que o fold existe
  para evitar (RFC-0024 §0: fold get não é
  linearizável; lab get LocalApplied p50 ~0.07
  ms vs put ~246 ms). Freshness do paper é **~1 s**
  (Table 4), não µs. Quem quiser SI vai a
  `get_strong` no Montanha, não ao fold.
- **Não é “TiFlash = fold”.** Partilham o *papel*
  Raft (1). Diferem no formato (2) e na semântica
  de read (3). O hook do catálogo (“learner =
  fold/column path”) fica **demasiado largo** —
  corrigido em `correlacao/`.
- **Não implementar:** learner colunar, DeltaTree,
  coprocessor, TiSpark, 500 MB cap, PD oracle.
- **Teste que já temos e o paper abona:** fold
  crash/resume sem o processo ser voter
  (`fold_follow_montanha_prefix`,
  `fold_resume_after_reopen`). Um teste que
  *não* vamos escrever: fold que bloqueia em
  read-index.
- **Falsificaria L24:** um workload Caixote em que
  LocalApplied serve dado *errado* (não stale —
  *wrong*) e `get_strong` no request path ainda
  cabe no p99. O paper não mostra esse workload;
  mostra lag de até 1 s com SI. Stale ≠ wrong.

Fichas irmãs: R041 Percolator (D4, 2PC do engine);
R007 Dostoevsky (D4, outra “não uma policy”);
R090 Naiad (listed — vocab incremental do fold);
R043 FDB (listed); R045 CRDB (listed; p. 12
desta).

## Avaliação crítica

- **6 servidores, 10 GbE, 100 W ≈ 70 GB.** Isolation
  ≤10% e lag ~1 s são *deste* cluster, com TiKV e
  TiFlash em **máquinas distintas**. Não é um
  fold in-process.
- **“Real-time”** no abstract é o ~1 s da Table 4,
  não o p50 LocalApplied do lab Pedra.
- **Table 2** compara DeltaTree a *um* LSM com
  universal compaction *dentro do TiFlash*, não a
  Pedra/Rocks de produção. WA 16× “aceitável”
  para eles; L7/L3 já recusaram meter coluna no
  kernel.
- **Fig. 7 vs CRDB** é OLTP-only (sem TiFlash).
  Não prova o learner.
- **CRDB “no HTAP”** (p. 12) é 2020; não fichámos
  R045 ainda.
- **Percolator no engine** (p. 3) — não reler R041
  como se fosse TiKV.
- **Não medem:** learner como *voter acidental*;
  fold prefix (só tabela inteira); crash do
  learner a meio do replay (Table 1); DST.

## Palavras-chave

tidb; tikv; tiflash; raft; learner; quorum;
read-index; htap; isolation; freshness; percolator;
deltatree; region; ch-benchmark

## Fontes

**Primária:** `docs/references/tidb-raft-htap-vldb2020.pdf`
(PVLDB 13(12), 13 pp.) +
`docs/references/tidb-raft-htap-vldb2020.pages.txt`.

**Secundárias (não citadas como se fossem este paper):**
RFC-0024; `crates/pedradb-fold`; ficha R041;
`docs/htap-storage-primitives-and-research.md` (L7).
Naiad / FDB / CRDB **não** foram lidos nesta ficha.
