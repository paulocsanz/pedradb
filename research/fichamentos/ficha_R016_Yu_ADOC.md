# Fichamento: R016 — ADOC (FAST’23)

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R016`

## Referência

YU, Jinghuan; NOH, Sam H.; CHOI, Young-ri;
XUE, Chun Jason. **ADOC: Automatically Harmonizing
Dataflow Between Components in Log-Structured
Key-Value Stores for Improved Performance.** In:
*21st USENIX Conference on File and Storage
Technologies (FAST ’23)*, 21–23 February 2023,
Santa Clara, CA. pp. 65–80 (+ refs).
URL: https://www.usenix.org/system/files/fast23-yu.pdf
Código: https://github.com/supermt/FEAT_7.11 (ref. [3])

Locus = **p. USENIX** (65–80). Figs. 1–21 e Tables 1–4
conferidos no texto extraído. Barras de Figs. 3–8,
13–15, 17–21: números só do parágrafo quando o OCR
não dá a barra. 87.9% (stall vs Rocks-AT) **só** no
abstract / §1 / §8 — §6.2 reporta stall vs SILK-O,
não reconstitui os 87.9%.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra:
  abstract, §§1–8, Figs. 1–21, Tables 1–4.
  Referências [1]–[61] **não** foram relidas.
  SILK [7] / Monkey [13] / Dostoevsky [14] /
  Rocks Experience [15] / Endure [22] só pelo que
  *este* texto afirma (R017, R006, R007, R010,
  R014 já D4). MatrixKV [58], NoveLSM, SLM-DB,
  ListDB, SpanDB, KVell **não** lidos. SILK+
  (TOCS’20) **não** está aqui.
- **Arquivo lido:** `research/fontes/R016_Yu_2023_ADOC.pdf`
  + `.txt` + `.pages.txt`.
- Como foi lido: PDF local, seção a seção, nesta sessão.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 65–66).** Write stall = queda
súbita de throughput sob write pesado (Fig. 1:
fillrandom, quatro classes de dispositivo, pressão
crescente). Duas propriedades: **universal** (há
stall em PM, NVMe, SATA SSD e HDD) e **forte
dependência do dispositivo** (duração e ritmo
mudam). Estudos anteriores atribuem o stall a
*uma* causa (CPU, BW, L0–L1, compacto fundo).
ADOC diz: essas teses são válidas no setting
deles e **não generalizam**. A história que
unifica é *data overflow* — expansão rápida de
um componente porque o fluxo de entrada supera
o de saída. ADOC = tuner online de **nº de
threads** e **batch / memtable size** (~300 LOC
em Rocks). Abstract: até **87.9%** menos duração
de stall e **+322.8%** vs Rocks auto-tuned;
até **+66%** throughput vs SILK no sintético
write-intensive; YCSB comparável; SILK gasta
**>20%** mais DRAM (p. 66: **22.2%** em média).

**§2 Fundo (p. 66–67).** NVMe vs PM (Optane
descontinuado; CXL-SSD citado). Fig. 2 = Rocks:
active + immutable memtable, WAL, L0
sobreposto, L1+ disjoint, flush e compact.
L0–L1 **não** paraleliza com outro L0–L1.
Três stalls nativos (wiki Rocks [28]):

- **MT:** 2 memtables cheias → input pára.
  “most common case” (p. 67).
- **L0:** slow em **20** ficheiros, stop em
  **36** (LevelDB → herdeiros).
- **PS (pending input size):** slow **64 GB**,
  stop **128 GB**. Paper chama isto
  “redundant data”, não só bytes mortos —
  *todo* o SST pendente conta.

**§3 Observações (p. 67–70).** Servidor 2× Xeon
Gold 6230, 40 cores, 128 GB DRAM. Table 1:
Optane DC PMM 512 GB / 2300 MB/s; Samsung
970 PRO 1 TB / 2700 MB/s; Intel DC S4500
960 GB / 490 MB/s; Seagate ST1000DM010 1 TB /
210 MB/s. Ubuntu 18.04, **RocksDB 6.11**,
fillrandom 16 B + 1000 B, **1 hora**, pico.
Listener de eventos Rocks. Média de 3
corridas; as 3 levam **>240 h**. Knobs: threads
(¼ flush, resto compact, default Rocks) e
batch 64 vs 512 MB.

Table 2 — confirma e limita:

| Antes | Confirmam | Limitam |
|-------|-----------|---------|
| CPU / BW [C1–C3] | mais threads baixa CPU e, até um ponto, stall; PM/NVMe stallam menos que SATA | [L1] além de um nº de threads, CPU normalizada cai e **stall sobe**; [L2] batch 512 MB corta stall sem mudar CPU; [L3] PM/NVMe stallam **com BW ociosa** (Fig. 5) |
| L0–L1 [C4] | cedo, 2 threads: trough ≈ L0–L1 (Fig. 6a) | [L4] com o tempo, e com 20 threads, **não há** alinhamento (Fig. 6b) — stalls passam a ser MT |
| Compacto fundo [C5] | mais threads → mais compactos, taxa média cai (Fig. 7) | [L5] PS **desce** com mais threads e batch maior (Fig. 8) |

**§4 Data overflow (p. 70–72).** Definição e
três tipos (Fig. 9):

- **MMO** (Memory Overflow) → MT stall:
  input > flush do immutable.
- **L0O** (Level 0 Overflow) → L0 stall:
  flush > L0–L1.
- **RDO** (Redundancy Overflow) → PS stall:
  geração de redundância > compacto fundo.

Explica L1–L5: o sistema **aborta o input
antes** de saturar hardware (por isso CPU/BW
baixos no stall). Fig. 10 (NVMe, 4 threads,
600 s): picos de L0 SST = L0 stall; RDO aos
**64 GB** com CPU e disco **baixos e
estáveis**. 2 threads: L0–L1 alinha cedo,
desalinha quando compactos fundos competem
pelo único worker. 20 threads: flush
esfomeado → MMO frequente, L0O some.

**§5 ADOC (p. 72–73).** Princípios: (1)
**device-transparent** — olha o fluxo, não o
spec do disco; (2) **portável** — APIs nativas,
sem mudar a árvore. Fig. 11: Rocks preto +
extensões vermelhas. Só `Options` + classe
tuner. **~300 LOC** (250 tuner + 50 estado).
Janela **Tw = 1 s** (maior = tarde demais em
PM/NVMe; menor = ruído).

Acções (AIMD: +2 threads / +64 MB; descer
**a metade**; flush continua ¼):

| Overflow | Detecta | Threads | Batch |
|----------|---------|---------|-------|
| MMO | active enche antes do imm flushar | **desce** (mais BW ao flush) | **sobe** |
| L0O | L0 ≥ 20 | **sobe** (L0–L1 apanha worker; flush abranda) | **não mexe** (subir carrega L0–L1; descer gera mais L0) |
| RDO | redundância ≥ 64 GB | **sobe** | **desce** (jobs mais finos) |

Vários no mesmo Tw: **L0O > RDO > MMO**.

**§6.1 Setup (p. 73).** Hardware = §3.1. Batch
máx. 512 MB (como SILK). ADOC e Rocks em
**RocksDB 7.5.3**; SILK ficou em **5.7.1**
(port falhou). Table 3: RocksDB-DF (2 threads,
64 MB), RocksDB-AT (rate-limiter auto),
SILK-D (default Rocks), SILK-P (paper: 4
threads, 128 MB), SILK-O (óptimo *manual*
neste lab: **8 threads, 512 MB**, grelha
2…20 × {256, 512} MB), ADOC. SILK: ¼ da BW
do media para compact, resto flush; **L0O
desligado** (limiar L0 enorme).

**§6.2 Micro (p. 73–75).** fillrandom, 3600 s.
Fig. 12 médias (kOps/s, 3 runs):

| | DF | AT | SILK-D | SILK-P | SILK-O | ADOC |
|--|---:|---:|-------:|-------:|-------:|-----:|
| PM | 63.6 | 50.0 | 46.9 | 68.3 | 126.9 | **211.5** |
| NVMe | 40.9 | 47.2 | 35.9 | 53.8 | 85.1 | **117.4** |
| SATA SSD | 28.4 | 32.3 | 22.3 | 29.6 | 53.0 | **69.5** |
| HDD | 17.0 | 14.3 | 11.5 | 11.6 | 19.0 | **29.5** |

ADOC vs SILK-O: **+66.7 / 37.8 / 31.0 /
55.1%**. 211.5 / 50.0 ≈ **+323%** vs AT no
PM — casa com o 322.8% do abstract. Rocks-AT
precisa que o user dê a BW; ganha a DF só em
flash.

Fig. 13 stall vs SILK-O: PM **−45.2%**, SATA
SSD **−8.7%**, HDD **−10.3%**; NVMe
**+1.5%**. MT e L0O: ADOC o mais baixo. PS:
ADOC *pior* no mesmo relógio de 3600 s —
porque aceita **+66.9 / 46.9 / 46.5 / 53.5%**
mais input (Fig. 14). HDD: +53.5% input,
espaço só **+19.4%**, CPU **+72.5%**,
contagem de ops **+143.6%**. SILK *adormece*
compactos sob congestionamento; ADOC **mantém
≥1 compacto sempre a correr**.

Fig. 16 p99 vs SILK-O: ADOC melhor no PM;
**+70.1 / 131.2 / 242.9%** em NVMe / SATA /
HDD. O paper atribui ao volume extra (mais
GC de primeiro plano, mais conflito no bus).
SILK *era* um paper de p99; ADOC é de
throughput / duração de stall.

**§6.3 YCSB (p. 75–76).** Table 4: A–F
clássico. Load 50 M (10 B + 1000 B); cada
run 1 h. ADOC vs Rocks-AT: **+7.6–11.4%**.
ADOC ganha o Load; SILK-O ganha A/B/C/F;
D/E depende do disco. ADOC-5.7.1 (mesmo
Rocks que o SILK) ganha na maioria dos runs,
sobretudo A e E no HDD — o salto 5.7→7.5
não é a favor do ADOC. Fig. 18: SILK-O até
**+76.8%** de RAM vs ADOC. p99 YCSB “não
mostra surpresas”: SILK-O no geral à frente.
Conclusão do paper: comparáveis; SILK-O
exige afinação *manual*; ADOC é online.

Read-while-writing (50 M write + 50 M read):
ADOC **+5.2–45.3%** vs SILK-O, **+95–135%**
vs Rocks-AT (Fig. 19).

**§6.4 Ablation (p. 76).** ADOC-T (só
threads) bom em PM, piora em disco lento.
ADOC-B (só batch) bom em HDD, não usa o
paralelismo do NVMe/PM. Os dois juntos
estabilizam a curva (Fig. 21).

**§7 Related (p. 76–77).** SILK = prioridades
+ rate limiter, foca p99 / L0–L1 a tempo.
Luo/Carey [45, 50] alinham compactos.
MatrixKV: L0–L1 em PM + column-compaction.
NoveLSM / SLM-DB / ListDB = PM. P2KVS,
KVell, SpanDB = software/IO stack. Monkey /
Dostoevsky / Rafiki / TiKV-DNN / Endure =
tuning *offline* ou robusto — Endure
“Nominal Tuning Problem”. ADOC é o tuner
*online* de dois knobs nativos.

**§8 (p. 77).** Overflow explica o que as
teses de uma causa não cobrem. Harmonizar
fluxo > esperar o Rocks consumir o backlog.
Números do abstract repetidos. SILK +22.2%
DRAM.

## Tese central e argumento

Stall não é (só) “falta CPU”, “falta BW”,
“L0–L1” ou “compacto fundo”. É **descompasso
de caudal** entre input, flush, L0–L1 e
compacto fundo. Os três stalls nativos do
Rocks **são** os três overflows (MMO / L0O /
RDO) e disparam *antes* do hardware saturar
— por isso as teses de recurso falham em
PM/NVMe. Controlar o caudal com threads +
batch (AIMD, 1 s) corta overflow sem nova
árvore e sem o scheduler de I/O do SILK.

O paper **não** pede 16 compact threads, nem
Lazy Leveling, nem desligar o WAL. Pede
dois knobs que o Rocks já expõe ao vivo.

## Estrutura do texto

| § | p. | Conteúdo |
|---|---:|----------|
| Abstract / 1 | 65 | stall universal; overflow; 87.9 / 322.8 / 66 |
| 2 fundo | 66 | NVMe/PM; Fig. 2; MT / L0 / PS |
| 3 estudo | 67 | Table 1–2; Figs. 3–8; C1–C5 / L1–L5 |
| 4 overflow | 70 | MMO / L0O / RDO; Fig. 9–10 |
| 5 ADOC | 72 | Fig. 11; Tw; AIMD; ordem L0O>RDO>MMO |
| 6.1–6.2 | 73 | Table 3; Fig. 12–16 |
| 6.3–6.4 | 75 | YCSB; ablation |
| 7–8 | 76 | related; conclusão |

## Conceitos-chave

- **Data overflow.** Expansão rápida de um
  componente porque o inflow supera o
  outflow (p. 65, 70). Termo do paper.
- **MMO / L0O / RDO.** Overflow de memória,
  de L0, de redundância. Mapeiam 1-1 para
  MT / L0 / PS.
- **PS / “redundant data”.** Pending
  compaction *input* (64 / 128 GB) — não
  só bytes mortos (p. 67).
- **Device transparency.** Tunar o fluxo, não
  o modelo do disco (p. 72).
- **AIMD nos knobs.** +2 threads / +64 MB;
  descer a metade (p. 73).
- **Rocks-AT.** Rate-limiter auto [1, 39]:
  sobe BW com *backlog interno* — o mesmo
  enviesamento que SILK critica.
- **SILK-O vs SILK-P.** “Óptimo” *deste* lab
  (8 / 512 MB) ≠ paper (4 / 128 MB). A
  configuração mexe muito (Fig. 12).

## Citações relevantes

1. "data overflow, which refers to the rapid expansion of one or more components in an LSM-KV system due to a surge in data flow into one of the components" (p. 65)
2. "ADOC reduces the duration of write stalls by as much as 87.9% and improves performance by as much as 322.8% compared with the auto-tuned RocksDB." (p. 65)
3. "ADOC achieves up to 66% higher throughput for the synthetic write-intensive workload that we used, while achieving comparable performance for the real-world YCSB workloads." (p. 65)
4. "SILK attains this performance at the expense of using 22.2% more main memory on average." (p. 66)
5. "write stalls are universal, that is, they occur on all types of devices" (p. 66)
6. "write stalls are strongly device dependent, with their duration and rate of performance degradation influenced by various factors such as device type and write intensity." (p. 66)
7. "the default pending compaction input size threshold for slowing down and stopping the system is 64GB and 128GB, respectively." (p. 67)
8. "by default, a quarter of the threads are allocated for flush jobs (rounded down), while the rest perform compaction." (p. 68)
9. "write stalls may still occur before its bandwidth capacity is reached." (Table 2 [L3], p. 68)
10. "Correspondence between performance troughs and L0-L1 compaction jobs diminishes over time, especially in the multi-threaded environment." (Table 2 [L4], p. 68)
11. "Data overflow refers to the rapid expansion of one or more components in an LSM-KV system due to a surge in data flow into one of the components." (p. 70)
12. "all three kinds of data overflow will stop or slow down the input before the system reaches the hardware limitation." (p. 71)
13. "In total, we add around 300 lines of code (LOC) to implement ADOC with 250 LOC for the tuner and 50 LOC to collect system states." (p. 72)
14. "If multiple overflows are detected in Tw, we choose to handle the overflow in L0O, RDO, MMO order" (p. 72)
15. "It shows 66.7%, 37.8%, 31.0%, and 55.1% higher average throughput over the next best performing scheme, SILK-O, for PM, NVMe SSD, SATA SSD, and SATA HDD devices, respectively." (p. 73)
16. "ADOC reduces the stall duration compared to SILK-O for PM, SATA SSD, SATA HDD by 45.2%, 8.7%, and 10.3%, respectively. However, for NVMe SSD, stalls seem to be elongated by 1.5%." (p. 74)
17. "Compared to SILK-O, ADOC does better for PM, but is higher by 70.1%, 131.2%, and 242.9% for NVMe SSD, SATA SSD, and SATA HDD, respectively." (p. 75, p99)
18. "SILK-O uses as much as 76.8% more memory than ADOC." (p. 75)
19. "ADOC achieves 5.2% to 45.3% higher throughput than SILK-O, and 95% to 135% higher than RocksDB-AT." (p. 76, read-while-writing)
20. "the number of threads and batch size have a complementing effect that stabilizes the tuning process" (p. 76)

## Diálogo teórico

- **SILK [7] / R017** — irmão, não substituto.
  SILK = scheduler de I/O (prioridade, BW
  oportunista, preempt); ADOC = knobs de
  caudal (threads + batch). ADOC ganha
  fillrandom; SILK ganha p99 em 3/4 discos
  e usa mais RAM. SILK-O aqui é *manual*;
  o paper SILK não entregava esse 8/512.
- **Rocks-AT [1, 39]** — mesmo gênero
  “auto”, outro eixo (rate-limiter cego ao
  cliente). Baseline do +322.8%.
- **MatrixKV [58]** — L0–L1 como causa em
  PM; ADOC Table 2 [L4] recusa
  generalizar. Muda formato SST / árvore;
  ADOC não.
- **Luo & Carey [45, 50] / R012 cousin** —
  compacto fundo vs flush; ADOC [C5]/[L5]:
  PS não sobe com threads.
- **Monkey [13] / Dostoevsky [14] / Endure
  [22] / R006, R007, R014** — tuners
  *offline* / robustos de (T, bits, L|T).
  ADOC é online de *outros* dois knobs.
  Endure “Nominal = overfit”; ADOC não
  discute bola KL.
- **bLSM / LevelDB stalls [17, 28]** —
  vocabulário MT/L0/PS que ADOC rebatiza
  como overflow.

## Relação com Pedra / Montanha

Pedra `Db`: flush no caminho `&mut`
(writer pára — isto *é* MT stall, sem
nome). `ConcurrentDb`: write-group + dual
memtable + flush **single-flight**; compact
ainda exclusivo no install. Sem pool
flush/compact, sem rate-limiter, sem
L0-slow/stop, sem PS 64/128 GB.
`L0_COMPACTION_TRIGGER=4` dispara compact,
**não** stall. `auto_flush_bytes` default
**4 MiB**, não 64/512 MB. `MAX_LSM_LEVEL=3`.
`DbStats` não tem `stall_*`.

| ADOC (paper) | Pedra | Ledger |
|--------------|-------|--------|
| MT / L0 / PS = MMO / L0O / RDO | sem nomes; MT é o `&mut` flush | **L41 `MEASURE`** ganha a taxonomia. Primeiro slice = contadores |
| Tuner AIMD threads+batch, Tw=1 s | 0 knobs vivos (1 flush, compact síncrono, 4 MiB fixo) | **L42 `REFUSE`** tuner agora. Reabrir só se L41 nomear overflow *e* existirem ≥2 knobs (eco L12) |
| Device-transparent | DST precisa de knobs *desligáveis* | retune live é hostil ao sim (não-determinístico) |
| SILK complementar (p99 vs ops/s) | DCS/leases sentem p99 | **não** troca L41 por L42. SILK ≠ ADOC |
| L0-stop 20/36; PS 64/128 GB | não temos | **não** ligar stop/slow sem métrica (já na ficha R017) |
| ¼ threads = flush | sem pool | não inventar 16 bg (R013) |
| Rocks 6.11 estudo / 7.5.3 eval; SILK 5.7.1 | outro binário | números não transferem |

- **Não implementar** o tuner ADOC, AIMD,
  Tw=1 s, retune de memtable, nem portar
  FEAT_7.11. **Não** reabre L3/L37/L36
  (não é policy de merge). **Não** é
  motivo para 16 threads.
- **Primeiro slice se L41:** os nomes que
  ADOC usa — `stall_memtable` (MMO),
  `stall_l0` (L0O), `stall_pending` (RDO)
  — mesmo que os dois últimos fiquem 0
  para sempre (Pedra não tem esses
  limiares).
- **Teste que escreveríamos se
  roubássemos a ideia:** soak fillrandom
  1 h com dual-mem; se MMO>0 e L0O=0,
  a acção ADOC seria *descer* trabalho
  paralelo de compact (Pedra já tem 1
  compact). Se isso já é o caso, o tuner
  é no-op.
- **Falsificaria L42 (não fazer o tuner):**
  o soak L41 com p99 ≥10× p50 *e*
  contadores = WAL sync, não overflow.
  **Falsificaria “nunca ADOC”:** o mesmo
  soak com MMO/L0O/RDO nomeados *e* duas
  superfícies vivas (ex. `flush_threads`
  + `auto_flush_bytes`) em que um estático
  perde >2× para um AIMD `Env`-off no DST.

Fichas irmãs: R017 (p99 / scheduler);
R014 (tuner ≠ estes knobs; sem online);
R012 (policy de merge, outro eixo);
R013 (16 bg misturam níveis — exactamente
o MMO por flush esfomeado da Fig. 8);
R010 (Rocks ops: espaço>WA, não stall).

## Avaliação crítica

- **87.9%** não reaparece no §6 com
  dispositivo. Usar só como tecto do
  abstract. O número auditável vs SILK-O
  é 45.2 / 8.7 / 10.3 / **+1.5%** (NVMe).
- **+322.8%** casa com Fig. 12 PM
  211.5 vs AT 50.0. É o *melhor* disco ×
  o *pior* auto-tuner (AT no PM é pior
  que DF: 50.0 vs 63.6). Não é a mediana.
- **SILK em 5.7.1, ADOC em 7.5.3.**
  YCSB com ADOC-5.7.1 muda o vencedor.
  Comparar “ADOC vs SILK-O” no
  fillrandom mistura versões. SILK-O é
  *exaustivo neste lab*, não o artefact
  do paper SILK.
- **SILK corre com L0O off.** ADOC usa
  o limiar 20. Superfícies de stall
  diferentes.
- **WAL / compressão** do SILK original
  estavam off; ADOC §6 não declara o
  mesmo com a mesma clareza. Pedra
  default `sync=true` — p99 pode ser
  fsync, eixo que nenhum dos dois
  isola aqui.
- **p99:** ADOC *perde* para SILK-O em
  NVMe/SATA/HDD no micro. O abstract
  lidera com throughput. Quem lê só o
  abstract “66% better than SILK” perde
  a Fig. 16.
- **Não medem:** 1-writer Pedra-class;
  DST / retune sob seed; dual-mem sem
  thread pool; L≤3 Full-do-par; soak
  com fsync. Optane está morto; o “PM”
  não é o disco do P0.

## Palavras-chave

adoc; write-stall; data-overflow; mmo;
l0o; rdo; threads; batch-size; aimd;
rocksdb; silk; auto-tune; fillrandom

## Fontes

**Primária:** `research/fontes/R016_Yu_2023_ADOC.pdf`
(USENIX FAST ’23) + `.txt` + `.pages.txt`.

**Secundárias (não citadas como se fossem
este paper):** `concurrent.rs` (write-group /
flush single-flight); `db.rs`
(`L0_COMPACTION_TRIGGER`, `auto_flush_bytes`);
ficha R017; `00_estrategias.md` §1.5.
