# Fichamento: R012 — LSM Compaction Design Space

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R012`

## Referência

SARKAR, Subhadeep; STARATZIS, Dimitris; ZHU, Zichen;
ATHANASSOULIS, Manos. **Constructing and Analyzing the LSM
Compaction Design Space.** *PVLDB* 14(11):2216–2229, 2021
(VLDB ’21). DOI 10.14778/3476249.3476274.
URL: https://www.vldb.org/pvldb/vol14/p2216-sarkar.pdf
Artefactos: https://disc.bu.edu/lsm-compaction

Locus = **p. interna do PDF** (1–14 = PVLDB 2216–2229).
Figs. 1–8 e Tables 1–2 conferidos no texto extraído. Fig. 4–8
são barras; números tirados do parágrafo que as comenta, não
do raster.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract,
  §§1–7, Figs. 1–8, Tables 1–2. Referências [1]–[63] **não**
  foram relidas; Monkey [21] / Dostoevsky [23] / Lethe [51]
  / PebblesDB [46] / SILK [11] só pelo que *este* texto
  afirma (R006 e R007 já D4; Lethe R031 e Spooky R013
  listed). Tutorial R100 **não** lido.
- **Arquivo lido:** `research/fontes/R012_Sarkar_2021_LSMCompaction.pdf`
  + `.txt` + `.pages.txt`.
- Como foi lido: PDF local, seção a seção, nesta sessão.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 1–2).** Compactação = reorganizar runs
em níveis de capacidade \(T^i\). Decide WA, throughput de
write, point/range, space-amp e *delete*. O espaço é “vast,
largely unexplored, and has not been formally defined”;
produção escolhe uma strategy à mão. Contribuição 1:
**quatro primitivas** que definem qualquer strategy —
trigger, data layout, granularity, data movement policy.
Contribuição 2: **10 strategies** no mesmo RocksDB,
“more than 2000 experiments”, 12 observações + 7 takeaways.

Fig. 1: (a) RocksDB / Cassandra / X-Engine / Lethe ocupam
cantos diferentes do triângulo write / range / point / space
/ delete; (b) taxonomia das 4 primitivas.

Takeaways A–C (p. 2): **A** não há strategy perfeita;
**B** abrir a black-box em primitivas; **C** trocar de
strategy *pode* ganhar muito — o paper mede implicações,
não um auto-tuner.

**§2 Background (p. 2–3).** Buffer → flush de run; Level 0
= mem no modelo deles (Level 1…\(L-1\) no disco — convenção
≠ Rocks L0-on-disk). Compact = sort-merge Level \(i\) com
overlap de \(i+1\). WA “as high as **40×**” (PebblesDB
[46]). **Partial** = ficheiro(s) em vez de nível (Fig. 2);
não muda o I/O total worst-case, amortece picos. Point =
primeiro hit + Bloom/fences. Range = sort-merge de runs.
Delete = tombstone; persistente só quando chega ao último
nível.

**§3 Design space (p. 3–5).** Quatro perguntas:

1. **Trigger (quando).** Level saturation (bytes/capacidade);
   #sorted runs; file staleness; space-amp; tombstone-TTL
   (Lethe).
2. **Data layout (como ficar).** Leveling (1 run/nível);
   tiering (vários runs); **1-leveling** (tier no 1.º disco,
   resto leveled — default Rocks); **L-leveling** (leveled
   só em \(L\), Dostoevsky); hybrid por nível (LSM-Bush).
3. **Granularity (quanto).** Level inteiro; sorted runs;
   um ficheiro; vários ficheiros.
4. **Data movement (qual).** Round-robin; least-overlap
   parent / grandparent; coldest; oldest; tombstone
   density; tombstone-TTL.

Cardinalidade “plugging in some typical values … **>10⁴**”
(p. 4). Table 1 mapeia >20 sistemas. Ensemble: trigger /
granularity / movement são multi-valor; layout é
single-valor.

**§3.2 / Table 2 (p. 5–6).** Dez strategies medidas:

| Nome | Layout | Gran. | Pick |
|------|--------|-------|------|
| Full | leveling | levels | N/A |
| LO+1 | leveling | file | least overlap parent |
| LO+2 | leveling | file | least overlap grandparent |
| RR | leveling | file | round-robin |
| Cold / Old | leveling | file | coldest / oldest |
| TSD / TSA | leveling | file | tombstone density / age (+ LO+1) |
| Tier | tiering | runs | N/A (SA + #runs) = Rocks *universal* |
| 1-Lvl | hybrid | runs@L1 + files | least overlap parent |

**§4 Método (p. 5–6).** Fork **RocksDB v6.11.4**; compact
*acima* de writes; 1 thread de compact; 2 memtables;
direct I/O. Métricas: compact latency, WA, write tail,
RA, point/range, SA, delete persistence. Workload-gen
⊃ YCSB + deletes (GitHub [52]).

**§5 Setup (p. 6–7).** AWS **t2.2xlarge**, 8 vCPU 3.0 GHz,
32 GB, Ubuntu 20.04, SSD **io2 4000 IOPS** (40 GB; 500 GB
se data >16 GB). Default: \(T=10\), write buffer **8 MB**,
página **16 KB**, Bloom **10 bits/entry**, block cache
**8 MB**, 10 M inserts, KV **128 B** (key 4 B), uniforme
salvo senão dito. TSD/TSA omitidos sem deletes (caem em
LO+1).

**§5.1 Ingest / query (p. 7–9).**

- **O1.** Full move **63×** o ingest (32× read + 31× write);
  Tier **23×**; 1-Lvl ≈ partial leveled.
- **O2.** Partial: **34–56%** menos movimento que Full;
  LO+1/LO+2 **10–23%** menos que outros partial;
  **4×** mais jobs (= \(L\)). Pseudo-compact (sem overlap)
  = só metadata.
- **TA I.** Full faz \(\sim 1/L\) dos jobs mas \(\sim 2L\)
  mais dados por job.
- **O3.** Full mean latency **1.2–1.9×** partial leveling,
  **2.1×** tiering. CPU **~50%** em todas (merge em RAM
  domina). Tail mais previsível no Full; partial/tier
  variam com o pick.
- **TA II.** Tail write Tier **~25 ms** vs Old **1.3 ms**
  (5–12×; 2.5× vs Full).
- **O4.** Point: Full melhor, Tier pior
  (1.1–1.9× existing, ~2.2× empty). \(T\times\) de
  textbook **não** aparece — Rocks-tiering tem menos
  runs que o modelo. Full 3–15% abaixo do partial
  (níveis às vezes vazios).
- **TA III.** Com Bloom 10 bits + cache pequena, o
  *movement policy* **não** muda point — só o nº de runs.
- **O5.** RA \(\alpha=0\): 3.5–4.4; \(\alpha=0.8\):
  **14.4** leveling / **21.3** tiering. Range ~igual
  entre leveled; Tier **~5%** pior.
- **O6 / TA IV.** Mix: mean compact ≈ insert-only; tail
  write **2–15×**. Lookups interleaved **26–63%** (não
  vazios) / **69–81%** (vazios) mais rápidos que seriais
  — compact *aquece* filter/index/data no block cache.
  1-Lvl “best of both worlds” no mix.

**§5.2 Workload (p. 9–11).** Ingest uniforme / PrefixZipf
/ normal: movimento de compact **agnóstico** à
distribuição se ela for estacionária (O7). Lookup Zipf:
todas as strategies iguais (cache chega). Lookup normal:
LO+1/LO+2 ganham; LO+2 imprevisível. **O8 / TA VI:**
update-heavy → Tier *domina* (descarta mais por job);
LO+2 melhor entre leveled (~20% menos dados).
**Deletes (O9 / TA VII):** TSD purga **16%** mais
tombstones que Tier e **5%** mais que LO+1 (10% deletes);
TSA 7–10% menos tombstones que TSD, mas
TSD/TSA50 movem **18%** mais que LO+1 (TSA33:
**35%**). **O10:** Tier escala mal; vantagens do
universal Rocks **somem >8 GB**; tail sobe. **O11:**
entry pequena (mais keys/página, Bloom maior) encarece
leveling.

**§5.3 Tuning (p. 11–12).** **O12:** buffer maior →
ficheiros maiores → mais dados/job; Tier escala *melhor*
com buffer (tail cruza as eager a **64 MB**). Page size
afecta todas na mesma direcção. \(T\), bits de Bloom e
tamanho de cache **não** mudam o *ranking* das
strategies.

**§6–7 (p. 12).** Modularizar primitivas no Rocks;
evitar Tier se o SLA é tail; evitar LO+2 se quer
estável; 1-Lvl = mais estável. HTAP com localidade:
cache pode tornar optimizações de read “caras”
desnecessárias. Visão: primitivas *por nível* e
auto-select — **não medido**.

## Tese central e argumento

Compactação não é um binário leveling/tiering. É um
ponto num espaço de **quatro primitivas** (p. 1–4).
Nenhuma strategy ganha em WA + tail + point + range +
space + delete ao mesmo tempo (A, Fig. 8 caption). O
paper **não** pede um menu de 10 policies no motor;
pede vocabulário para *não* tratar compact como
black-box e para não copiar “tiering = writes” sem
olhar ao tail (p. 12).

## Estrutura do texto

| § | p. | Conteúdo |
|---|---:|----------|
| Abstract / 1 | 1 | 4 primitivas; 10 strategies; A–C |
| 2 background | 2 | LSM, partial, tombstone |
| 3 space | 3 | trigger / layout / gran. / pick; Table 1 |
| 3.2 / Table 2 | 5 | as 10 medidas |
| 4 método | 5 | Rocks 6.11.4; métricas |
| 5.1 | 7 | Fig. 4–5; O1–O6; TA I–IV |
| 5.2 | 9 | Fig. 6–8; O7–O11; TA V–VII |
| 5.3 | 11 | O12; buffer / page / \(T\) |
| 6–7 | 12 | pitfalls; 1-Lvl; não auto-tuner |
| refs | 13 | [1]–[63] |

## Conceitos-chave

- **Quatro primitivas.** Trigger, data layout, granularity,
  data movement. Termos do paper (p. 2). O catálogo
  Pedra dizia when / which / how much / layout — o
  paper põe *layout* e *which* (movement) como eixos
  distintos.
- **1-leveling.** L1 disco tiered, resto leveled. Default
  Rocks (p. 4, 6).
- **L-leveling.** Leveling só em \(L\) = Dostoevsky no
  mapa (Table 1). **Não** é uma das 10 medidas.
- **Full vs partial.** Granularity nível vs ficheiro
  (Fig. 2). Partial não muda I/O worst-case (p. 3).
- **Pseudo-compaction.** Ficheiro sem overlap: só
  pointer (p. 7).
- **Universal.** Rocks-tiering disparado por space-amp
  + #runs (p. 5, Table 2).
- **TSD / TSA.** Pick por densidade / idade de
  tombstone (Lethe [51]).
- **\(\alpha\).** Fracção de point lookups vazios (p. 8).

## Citações relevantes

1. "four design primitives that can formally define any compaction strategy: (i) the compaction trigger, (ii) the data layout, (iii) the compaction granularity, and (iv) the data movement policy." (p. 1)
2. "There is no perfect compaction strategy." (p. 2)
3. "write amplification, which can be as high as 40× in state-of-the-art LSM-based data stores" (p. 3, [46])
4. "partial compaction does not radically change the total amount of data movement due to compactions, but amortizes this data movement uniformly over time" (p. 4)
5. "we estimate the cardinality of the compaction universe as >10^4, a vast yet largely unexplored design space." (p. 4)
6. "Full moves 63× (32× for reads and 31× for writes) the data originally ingested." (p. 7)
7. "The data movement is significantly smaller for Tier, however, it remains 23× of the data size." (p. 7)
8. "leveled partial compaction leads to 34%–56% less data movement than Full." (p. 7)
9. "Full-level compactions perform about 1/L times fewer compactions than partial compaction routines, however, full-level compaction moves nearly 2L times more data per compaction." (p. 7, TA I)
10. "Full compactions have the highest average latency (1.2–1.9× higher than partial leveling, and 2.1× than tiering)." (p. 7)
11. "The CPU effort is about 50% regardless of the compaction strategy." (p. 7)
12. "Tail write stall for Tier is ∼25ms, while for partial leveling (Old) it is as low as 1.3ms." (p. 7, TA II)
13. "the mean latency for point lookups with tiering is between 1.1–1.9× higher than that with leveled compactions for lookups on existing keys, and ∼2.2× higher for lookups on non-existing keys." (p. 8)
14. "The point lookup latency is largely unaffected by the data movement policy." (p. 8, TA III)
15. "the read amplification across different compaction strategies for non-empty queries (α = 0) is between 3.5 and 4.4" / "reaching up to 14.4 for leveling and 21.3 for tiering (for α = 0.8)" (p. 8)
16. "the tail write latency is increased between 2–15×" (p. 9, mix)
17. "When subject to update-intensive workloads, Tier exhibits superior compaction performance along with comparable lookup performance (as leveled LSMs)" (p. 10, TA VI)
18. "For a workload with 10% deletes, TSD purges 16% more tombstones than Tier and 5% more tombstones than LO+1" (p. 10)
19. "TSD and TSA50 compacts 18% more data than the write optimized LO+1 (for TSA33 this becomes 35%)." (p. 11)
20. "the advantages of the RocksDB-implementation of tiering (i.e., universal compaction) diminishes as the data size grows beyond 8GB." (p. 11)
21. "applications requiring stable performance should avoid LO+2 due to its unpredictable performance." (p. 12)
22. "partial compactions with leveling, and especially, hybrid leveling (e.g., 1-Lvl) offer the most stable performance." (p. 12)

## Diálogo teórico

- **O’Neil LSM [45]** — a árvore.
- **Monkey [21] / R006, Dostoevsky [23] / R007,
  LSM-Bush [24]** — layout L-leveling / hybrid no
  mapa (Table 1). Este paper **não** reabre L3: LL
  não está nas 10 medidas.
- **Lethe [51] / R031** — agora ficha D4. TSD/TSA = FADE
  SD/DD. L38 MEASURE (depois L35); L43 KiWi REFUSE.
- **PebblesDB [46]** — WA 40×; fragmented LSM.
- **SILK [11, 12]** — stalls; eles *invertem* a
  prioridade (compact > write) para isolar o efeito.
- **Rocks leveled [48] / universal [49]** — 1-Lvl e
  Tier.
- **Dong CIDR’17 [28] / R010** — space-amp; só citados.
- **Luo & Carey survey [42]** — background.
- **YCSB [19]** — o gerador é *superset*; o corpo
  **não** corre os six workloads nomeados.

## Relação com Pedra / Montanha

Pedra hoje (`db.rs`):

| Primitiva | Pedra | Ponto Table 2 |
|-----------|-------|----------------|
| Trigger | `auto_compact_sst_count` (L0 trigger = 4) e/ou `auto_compact_sst_bytes`; senão manual | file-count / bytes — **não** saturação \(T\) |
| Layout | níveis 0…`MAX_LSM_LEVEL=3`; flush em L0 | leveling *intencional*; L0 multi-ficheiro até compact |
| Granularity | **todos** os SST de \(N\) ∪ \(N+1\) → **um** SST em \(N+1\) | **Full** do par de níveis (não LO+1) |
| Movement | N/A (pega o nível todo) | como Full |
| Extra | `compact_for_reads` = latest-only de **todos** os SST num ficheiro em \(L_\max\) | mais Full ainda |

Bloom 10 bits = o default do paper. \(L\le 3\) ≠ árvore
\(T=10\) de 4 níveis do setup. RFC-0012 já recusou LL.

| Paper | Pedra | Ledger |
|-------|-------|--------|
| 4 primitivas = vocabulário | `compact_levels` já é um ponto; não um menu | **não** nova linha SHIP. Nomear no Relação basta |
| File-granular + LO+1 (−34–56% vs Full; −10–23% vs outros partial) | Pedra é Full-do-par | **L35 `MEASURE`**: um compact por ficheiro + least-overlap parent. Gate: soak de WA/stall em `benches/baseline` com \(L=3\). **Antes** de Spooky (L11) |
| Menu de 10 / auto-switch §6 | uma policy que o DST explica | **L36 `REFUSE`** |
| Tier / universal default | tail 25 ms, escala mal >8 GB, point pior | **L37 `REFUSE`** como default. TA VI (update-heavy) **não** chega: Pedra serve DCS/leases (tail) |
| TSD/TSA | tombstone existe; sem TTL pick | **L38 `MEASURE`** (ficha R031): FADE depois de L35, só com SLA Dth. KiWi = L43 REFUSE |
| L-leveling / Dostoevsky | L3 já REFUSE | **não reabre** (LL fora das 10; depende de L2) |
| 1-Lvl “mais estável” | L0 já acumula 4 ficheiros | **não** SHIP. Se L35 passar, 1-Lvl é o default Rocks a *comparar*, não a copiar às cegas |

- **Não implementar** as 10 strategies, universal, TSD,
  Cascades-of-compact, auto-tuner.
- **Teste se roubássemos L35:** mesmo workload de
  `leveled_compaction_produces_multi_level_shape` + um
  soak de overwrite: WA (bytes_written_sst / ingest) do
  Full-do-par vs pick do ficheiro de menor overlap;
  compact *nunca* deixa half-file (tmp→rename já existe);
  DST crash a meio igual ao compact actual.
- **Falsificaria L37:** um update-only nomeado em que
  Pedra perde >2× de ingest para um universal *e* o p99
  de write cabe no SLA Caixote. O paper mede Rocks
  6.11 + t2 + io2 4000 IOPS — não o nosso NVMe/embed.
- **Falsificaria L36:** um único extra-knob (não dez)
  que o DST reproduz por seed e que ganha um workload
  nomeado sem regressão nos outros. Isso é L35, não um
  menu.

Fichas irmãs: R007 (L3; “no single policy”); R006 (Bloom
10 bits); R010 (space>WA; compact dinâmico); R013 Spooky
(próximo se L35); R031 Lethe (se L38).

## Avaliação crítica

- **Rocks 6.11.4, 2020.** Compact Rocks mudou; números
  não são Pebble 2026 nem Pedra.
- **t2.2xlarge + io2 4000 IOPS.** CPU burstable; SSD de
  rede provisionada, não NVMe local. Tail de 25 ms é
  *deste* device.
- **10 M × 128 B ≈ 1.28 GB** no base. O10 vai a \(2^{35}\) B
  mas o ranking “agnóstico ao tamanho” (p. 7) **não**
  vale para Tier.
- **Compact > write, 1 thread, 2 memtables, direct I/O.**
  Isola o efeito; **não** é produção (SILK é o contrário).
- **Key 4 B.** Irrealista; muda prefix-overlap e portanto
  LO+1.
- **Fig. 4–8 sem tabela de barras.** Usei só números do
  texto. Não inventar alturas.
- **Não medem:** fluid \(K,Z\); Spooky; Pedra-class
  \(L=3\) whole-merge; transição *entre* strategies
  (mudar API a quente, p. 6, sem número).
- **“No perfect strategy”** confirma R007 Fig. 10B com
  *outro* eixo (primitivas, não só \(T,K,Z\)). Não é
  licença para 10 policies.

## Palavras-chave

lsm; compaction; design-space; trigger; data-layout;
granularity; data-movement; leveling; tiering;
1-leveling; partial; least-overlap; write-amp;
tombstone; rocksdb

## Fontes

**Primária:** `research/fontes/R012_Sarkar_2021_LSMCompaction.pdf`
(PVLDB 14(11), vldb.org) + `.txt` + `.pages.txt`.

**Secundárias (não citadas como se fossem este
paper):** RFC-0012; `crates/pedradb-core/src/db.rs`
(`compact_levels`, `MAX_LSM_LEVEL`, `L0_COMPACTION_TRIGGER`);
ficha R007; ficha R006; `correlacao/00_estrategias.md` §1.2.
R013 / R031 **não** lidos.
