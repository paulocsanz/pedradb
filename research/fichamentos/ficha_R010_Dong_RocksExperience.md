# Fichamento: R010 — RocksDB Experience (FAST’21)

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R010`

## Referência

DONG, Siying; KRYCZKA, Andrew; JIN, Yanqin; STUMM, Michael.
**Evolution of Development Priorities in Key-value Stores Serving
Large-scale Applications: The RocksDB Experience.** In: *19th
USENIX Conference on File and Storage Technologies (FAST ’21)*,
23–25 Feb. 2021. p. 33–50 (PDF interno 2–17 + apps A–C).
ISBN 978-1-939133-20-5.
URL: https://www.usenix.org/system/files/fast21-dong.pdf

Locus = **p. interna do PDF** (capa = p. 1; corpo começa p. 2 =
FAST 33). Tables 1–6 e Figs. 1–4 conferidas no texto extraído.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract, §§1–9,
  Figs. 1–4, Tables 1–6, Appendix A (timeline 2012–2020), B
  (15 lições), C (3 design choices revisitados). Referências
  [1]–[71] **não** foram relidas como obras; WiscKey [35] e
  Monkey [17] só pelo que o texto afirma (R005/R006 já
  fichadas).
- **Arquivo lido:** `research/fontes/R010_Dong_2021_RocksExperience.pdf`
  + `….txt` (pdftotext) + `….pages.txt` (pypdf). Citações
  conferidas no `.pages.txt`.
- Como foi lido: PDF local, seção a seção, nesta sessão.
  Fig. 3 é um conjunto de CDFs de utilização — números
  pontuais tirados do parágrafo, não digitalizados barra a
  barra.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 2–3).** RocksDB (2012, fork LevelDB) é
um **motor embed num só nó**: não faz replicação, load
balance nem checkpoints inter-host — deixa isso à
aplicação e oferece *support* (p. 2). >30 apps no Facebook,
centenas de PB. Usos (Table 1): DBs (MySQL/MyRocks,
Cassandra, CRDB, Mongo, TiDB), stream (Flink/Kafka/
Samza/Stylus), filas (LogDevice), índices (Dragon),
cache SSD (EVCache/Pika/Redis). Table 2: stream é
CPU 11% / space 48%; cache é space 78% / flash
endurance 74%. Prioridades migraram: write amp →
space amp → CPU. Lições: recurso *cross-instance*,
formato back/forward compatible, backups/repl,
corrupção detectada **cedo e em todas as camadas**.

**§2 Background (p. 3–4).** SSD: assimetria R/W,
endurance limitada, bottleneck muitas vezes na
*rede* → KV local. LSM + MemTable skiplist + WAL
opcional + SST + leveled / universal (tiered) / FIFO.
Table 3 (RocksDB 5.9, 20 bits/key no FIFO, cache
10%):

| Style | WA | space max | Get+bloom |
|-------|---:|----------:|----------:|
| Leveled | 16.07 | 9.8% | 0.99 I/O |
| Tiered | 4.8 | 94.4% | 1.03 I/O |
| FIFO | 2.14 | n/a | 1.16 I/O |

**§3 Resource targets (p. 5–6).** Começaram em WA
(community [34] SILT). Leveled WA 10–30; LinkBench
MyRocks escreve **5%** do InnoDB (p. 5). Tiered 4–10.
Depois: **espaço** é o bottleneck da maioria — IOPS
SSD sobram, HDD ainda não serve. Dynamic Leveled
(CIDR’17 [19]): overhead ~12–13% vs LevelDB-style
até 25% (Table 4), pior caso estático 90%. UDB:
espaço **50%** ao trocar InnoDB→RocksDB. CPU: não
acham que o SSD “ultrapassou o software”; 42
deploys ZippyDB/MyRocks (Fig. 3) são sobretudo
*space constrained*. CPU importa agora porque
DRAM/CPU ficaram caros e o low-hanging fruit do
espaço já foi colhido (prefix bloom, bloom-before-
index). Recusam open-channel/ZNS/in-storage como
alvo *unificado* (minoria das apps). Prioridade
actual: **storage remoto** + SCM por investigar.
LSM continua a escolha; para values grandes citam
WiscKey/ForestDB e dizem que estão a meter
**BlobDB** (p. 6).

**§4 Large-scale (p. 7–8).** Muitos shards = muitas
instâncias Rocks por host → limites globais
(mem, compact I/O/threads, disco, delete rate) e
locais. WAL em 3 modos: sync / buffered / **off** —
sistemas com log Paxos “não precisam” do WAL do
Rocks (p. 7). Delete rate-limited por causa de TRIM
spikes. Formato **forward compatible ≥ 1 ano**
(rollout/rollback mensal). Config: 39 ZippyDB
deploys, **>25 configs distintas** (Table 5);
arrependem-se do excesso de knobs; rumo a
adaptivity + out-of-box. Repl/backup: *logical*
scan (sem cache trash) ou *physical* copy de SST
(lock files). BackupEngine só se o requisito for
simples. Não abrem block device/FTL directo —
querem o FS para a app copiar ficheiros.

**§5 Failure (p. 8–10).** Corrupção a nível Rocks
(índice primário vs secundário MyRocks): **~1 / 3
meses / 100 PB**; **40% já tinha ido a outras
réplicas**. Transferência: ~17 mismatches / PB.
Fig. 4: checksums em 4 sítios — block (LevelDB),
file (2020), WAL handoff, per-KV (em curso).
Erros: não mais “halt all writes”; retry do que
for transiente (disco cheio, rede).

**§6 KV interface (p. 10–11).** KV chega para quase
tudo; 2PC *fora* do Rocks é ineficiente → TX
optimista/pessimista (MyRocks). Sequence de 56
bits interno **não** dá snapshot do passado nem
leitura cross-shard. Timestamp no key piora
point lookup; no value piora write fora de
ordem. API de user timestamp: Table 6, **1.2–2.0×**
vs timestamp empacotado na key (`fill_seq +
read_random` 1.2×; `fill_random +
read_while_writing` 2.0×).

**§7 Related (p. 11).** Library vs BerkeleyDB/
SQLite/Hekaton. LSM: WiscKey, Pebbles, Monkey
(“DRAM vs IOPS”), bLSM, TRIAD. Checksums =
end-to-end [55]. Timestamps: HBase/WT/Bigtable.

**§8–9 + Apps (p. 12–14).** Open: hybrid SSD/HDD,
delete-marker storms, write throttle, comparar
réplicas, SCM, integrity API para o FS. App. A:
timeline (Bloom 2013, backup/checkpoint 2014,
dynamic leveled 2015, file checksum 2020,
timestamps 2020). App. B: 15 lições. App. C:
revisitam “customizability is always good”,
“blind to CPU bit flips”, “OK to panic on any
I/O error”.

## Tese central e argumento

Tese (abstract + §3 + App. B): um LSM embed de
produção **não** é um paper de write-amp. O
gargalo real nas apps Facebook é **espaço**;
depois CPU/DRAM; WA só nas write-heavy. O motor
é um **library de um nó** — replicação, 2PC
distribuído e HTAP são da camada de cima. A
evolução útil é operacional: WAL configurável,
formato estável, checksums em camadas, poucos
defaults bons, não 200 knobs.

Isto **confirma** a doutrina Pedra (L9, RFC-0014
“utility and integrity before multi-TB write-amp
papers”) com locus de incumbente, não de blog.

## Estrutura do texto

| § | PDF | FAST | Conteúdo |
|---|----:|-----:|----------|
| Abstract / 1 | 2 | 33 | 8 anos, 30 apps, tese |
| 2 Background | 3 | 34 | LSM, Table 3 |
| 3 Resource targets | 5 | 36 | WA → space → CPU |
| 4 Large-scale | 7 | 38 | WAL, TRIM, config, backup |
| 5 Failure | 8 | 39 | 1/3mo/100PB, Fig. 4 |
| 6 KV interface | 10 | 41 | user timestamps |
| 7 Related | 11 | 42 | WiscKey, Monkey |
| 8–9 / A–C | 12 | 43 | open Qs, timeline, 15 lessons |

## Conceitos-chave

- **Single-node library.** Sem repl/LB inter-host
  (p. 2). Termo do paper.
- **Dynamic leveled compaction.** Tamanho do nível
  = f(último nível), não estático (Table 4).
- **WAL modes.** Sync / buffered / off (p. 7).
  Off só quando a *app* já tem log de consenso.
- **Forward compatibility ≥ 1 year.** Rollout
  mensal (p. 8).
- **Multi-layer checksum.** Block + file + handoff
  + KV (Fig. 4).
- **User timestamp.** Metadata fora de key/value
  (Table 6). ≠ timestamp oracle (R041/L23).
- **BlobDB.** KV-sep à WiscKey, em curso (p. 6).

## Citações relevantes

1. "each RocksDB instance manages data on storage devices of just a single server node; it does not handle any inter-host operations, such as replication and load balancing, and it does not perform high-level operations, such as checkpoints — it leaves the implementation of these operations to the application, but provides appropriate support so they can do it effectively" (p. 2)
2. "our priorities in developing RocksDB have evolved … resource optimization target migrated from write amplification, to space amplification, to CPU utilization" (p. 2, abstract)
3. "Leveled Compaction in RocksDB usually exhibits write amplification between 10 and 30" (p. 5)
4. "when running LinkBench on MySQL, RocksDB issues only 5% as many writes per transaction as InnoDB" (p. 5)
5. "for most applications, space utilization was far more important than write amplification, given that neither flash write cycles nor write overhead were constraining" (p. 5)
6. "Dynamic Leveled Compaction limits space overhead to 13%, while Leveled Compaction can add more than 25%" (p. 5, Table 4)
7. "Most of the workloads are space constrained." (p. 6, Fig. 3, 42 deploys)
8. "when object sizes are large, write amplification can be reduced by separating key and value (e.g. WiscKey [35] and ForrestDB [1]), so we are adding this to RocksDB (called BlobDB)" (p. 6)
9. "distributed systems often have their own replication logs (e.g., Paxos logs), in which case RocksDB WAL are not needed at all" (p. 7)
10. "across the 39 ZippyDB deployments we sampled, over 25 distinct configurations" (p. 8, Table 5)
11. "corruptions are introduced at the RocksDB level roughly once every three months for each 100PB of data. Worse, in 40% of those cases, the corruption had already propagated to other replicas" (p. 9)
12. "It is essentially impossible to create versions of data that offer cross-shard consistent reads" (p. 11; sequence interno)
13. "the application-specified timestamp API can lead to a 1.2X or better throughput gain" (p. 11, Table 6)
14. "a common complaint now is that there are far too many options and that it is too difficult to understand their effects" (p. 8)
15. "We continue to come to the conclusion that [LSM-trees] do" remain appropriate (p. 6)

## Diálogo teórico

No texto (não a biblio inteira):

- **LevelDB [22]** — fork e block checksum.
- **O’Neil LSM [45]** — estrutura que não
  abandonam (§2, §3 fim).
- **Dong CIDR’17 [19]** — dynamic leveled / espaço.
- **Cao FAST’20 [11] / R009** — workloads Facebook;
  este paper é o *follow-up de prioridades*.
- **WiscKey [35] / R005** — BlobDB (p. 6).
- **Monkey [17] / R006** — “DRAM vs IOPS” (p. 11);
  não adoptam a alocação como default.
- **PebblesDB [52], TRIAD [3], bLSM [57]** —
  mais WA-first do que eles agora.
- **Saltzer end-to-end [55]** — checksums.
- **TiDB [27] / R048** — listed como *user* do
  Rocks, não como HTAP.
- **CRDB [64], MyRocks [37]** — users.

## Relação com Pedra

O paper é o incumbente contra o qual RFC-0014 se
mede. Quase tudo que ele *prioriza* o Pedra já
escolheu por doutrina; o que ele *arrepende*
(knobs, panic-on-any-IO, WAL único) o Pedra
ainda pode evitar.

| Lição R010 | Pedra hoje | Ledger |
|------------|------------|--------|
| Library de um nó; repl é da app | L9, Montanha/fold | **L9 `SHIP` src+=ficha** |
| Espaço > WA na maioria SSD | 0026 pick C, `compact_vlog`, blobs 0029 | confirma L17; não reabre 0027/0028 |
| KV-sep para values grandes | L4 `SHIP` + 0029 | confirma; BlobDB ≠ HashKV |
| Bloom por SST | L1 `SHIP` | Table 3 assume bloom |
| WAL sync default; off só com log de consenso | RFC-0015 sync; L5b é *outra* recusa (vlog sem key) | **L25 `MEASURE`**: skip WAL no *apply* Montanha se o Raft log já é durável. Default Pedra **fica** sync. Não é L5b. |
| File + KV checksum | `verify_checksums` = SST+WAL CRC | **L26 `MEASURE`**: checksum de ficheiro no checkpoint/copy (eles viram 17 mismatches/PB no wire) |
| User timestamps | sequence interno + snapshot | **L27 `MEASURE`**: ≠ L23 oracle. Só se Montanha precisar de snapshot *cross-range* sem empacotar ts na key |
| Poucos knobs / adaptivity | OpenOptions já inchado; setter 0029 | **não** abrir mais knobs sem bench. Sem linha nova — doutrina RFC-0014 |
| LSM fica | sled-layer recusado | confirma |
| TRIM rate-limit | sem evidência de spike | sem linha até `compact` mostrar |

- **Não implementar nesta sessão.** L25/L26/L27 são
  MEASURE com gate: (L25) bench de double-log no
  `apply` Raft vs `sync:false` + crash table; (L26)
  checksum no `copy_file` do checkpoint; (L27) só
  depois de um produto nomear leitura cross-shard
  a um ts global.
- **L5b intacto.** “WAL off” do paper é *quando a
  app já tem Paxos*. “WAL off porque o vLog tem
  keys” continua recusado (R005).
- **L2 intacto.** Citam Monkey e não a promovem a
  default — alinhado com MEASURE.
- **Falsificaria L25:** um crash entre Raft commit
  e apply Pedra, com WAL off, sem silent-wrong.
  Sem esse DST, WAL local fica.

Fichas irmãs: R005 WiscKey (BlobDB); R006 Monkey
(citado, não shipped lá); R009 Cao FAST’20
(workload, listed); R048 TiDB (user Rocks).

## Avaliação crítica

- **Produção Facebook 2012–2020, SSD local.** Os
  “most workloads are space constrained” são 42
  deploys ZippyDB/MyRocks, não o lab Pedra de
  8 MiB. O *sentido* (espaço antes de WA
  exótica) transporta; os 10–30× de WA não.
- **RocksDB 5.9 na Table 3** — não é Rocks 2024
  (Ribbon, etc.).
- **“does not perform checkpoints” (p. 2)** vs
  BackupEngine / physical checkpoints na
  timeline 2014 (App. A). O abstract fala de
  checkpoint *inter-host*; o library *tem*
  checkpoint de ficheiros. Pedra
  `create_checkpoint` é o segundo.
- **1/3 meses / 100 PB** inclui CPU/memory
  bitflip detectado via índice primário ≠
  secundário. Não é taxa de CRC de disco.
- **Table 6** é db_bench, não produção.
- **Não medem:** Pedra-class `L≤4`, DST,
  NVMe local vs o remote storage que eles
  *começam* a priorizar.

## Palavras-chave

rocksdb; lsm; write-amplification; space-amplification;
wal; checksum; blobdb; dynamic-leveled; configuration;
single-node; facebook; fast2021

## Fontes

**Primária:** `research/fontes/R010_Dong_2021_RocksExperience.pdf`
(FAST ’21, USENIX open) + `.txt` + `.pages.txt`.

**Secundárias (não citadas como se fossem este paper):**
RFC-0014; RFC-0015 (WAL); RFC-0016; fichas R005/R006;
`Db::verify_checksums`. Cao FAST’20 (R009) **não**
foi lido nesta ficha.
