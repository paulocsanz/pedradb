# Fichamento: R023 — Disco (PACMMOD / SIGMOD’25)

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R023`

## Referência

ZHONG, Wenshao; CHEN, Chen; WU, Xingbo; ERIKSSON,
Jakob. **Disco: A Compact Index for LSM-trees.**
*Proc. ACM Manag. Data* 3(1), Article 33 (February
2025), 27 pages. SIGMOD ’25.
DOI: 10.1145/3709683
NSF PAR: https://par.nsf.gov/servlets/purl/10573375
ACM: https://dl.acm.org/doi/10.1145/3709683
Autor: https://wenshaozhong.com/

Locus = **p. PACMMOD** `33:1`–`33:27` (PDF 1–27).
Figs. 1–17 e Tables 1–2 conferidos no texto extraído.
Barras das Figs. 9–17: números só do parágrafo (OCR
das figuras ilegível). Ligaduras `fi`/`fl` partidas
no extract — citações recompostas a partir do
parágrafo. REMIX [53] / R022 **não** lido (só o
que *este* texto afirma).

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra:
  abstract, §§1–6, Figs. 1–17, Tables 1–2,
  Algs. 1–4. Referências [1]–[54] **não** foram
  relidas. REMIX [53], Monkey/Dostoevsky [16],
  SuRF/Rosetta/GRF [51, 36, 48], WiscKey [35],
  Bourbon [14], Maplets [12], R012 [43] só pelo
  que *este* texto afirma (R006, R007, R005, R012
  já D4). Relatório de reprodutibilidade SIGMOD’26
  **não** lido.
- **Arquivo lido:** `research/fontes/R023_Zhong_2025_Disco.pdf`
  + `.txt` + `.pages.txt`.
- Como foi lido: PDF local (NSF PAR, acmart
  PACMMOD 3(1) Art. 33, 27 p.), seção a seção.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 33:1–33:2).** Read LSM é
subóptimo porque os runs sobrepõem. Filtros
reduzem I/O mas **não localizam**; sob pressão
de memória o próprio BF vira bottleneck.
Disco = índice *multi-run* de **todas** as keys.
Representações compactas (bits discriminativos)
cortam comparações, cache misses e I/O em
point **e** range. Garantia: point e seek
fazem **no máximo 1 I/O aos runs** (eficiência
perto de B+). Melhora REMIX com partial keys;
persistir Disco é barato. Com o índice, a LSM
pode usar policy *write-efficient* e ainda ler
bem. Até **+220%** point/range vs RocksDB.

**§2 Motivação (p. 33:3–33:4).** DRAM/disco
encolhe (32 GB DRAM ≈ 4 TB HDD / 2 TB SSD →
1:128 / 1:64). Fig. 1 (simulação LRU, 256 MB,
8 runs × 32 MB, email keys + 120 B, BF 10
bits): B+ ≤2 I/O em qualquer cache; LSM só
cola no B+ com cache **>2%** do dataset (BF
cabe); abaixo disso **6×** mais I/O que B+.
Point com BF ainda ~80% do I/O de range.

Range = seek + next (iterador). Seek de key
inexistente = menor key ≥ search; precisa
seek **em cada run**. Range filters: interface
“existe key em [L,R]?”, não iterador; YCSB-E
= seek + 50 next (range aberto) — o filtro
só ajuda se search > max do run. Filtro
positivo em range longo não poupa o index
block.

REMIX [53]: índice esparso multi-run,
caminho pré-gravado, evita sort-merge on the
fly. Eficaz com cache farto ou Optane. Em
SSD + cache curto, I/O extra come a poupança
do merge: query ainda acede até **log(n)**
runs.

**§3.1 Árvore + segmentos (p. 33:5–33:6).**
Keys partidas em segmentos de comprimento
variável; âncora = lower bound (prefixo
mínimo único). B+ nas âncoras. Metadata do
segmento: **cursor offsets** (posição da
menor key de cada run no segmento), **run
selectors** (ordem de acesso — de REMIX),
**partial key mask** + **partial keys**.
Acesso aleatório à i-ésima key do segmento:
contar ocorrências do selector → 1 I/O.

**§3.2 Representação compacta (p. 33:6–33:11).**
Bits discriminativos = primeiro bit em que
vizinhos diferem. Mask marca essas posições
(≤ n−1 uns para n keys). Partial key =
PEXT da mask. Alg. 1: LCP de pares
adjacentes. Keys prefixo-uma-da-outra sem
bit 0 extra → mesmo partial key (tratadas
como iguais).

Point (Alg. 2): extract → exact match nos
partials. Sem match → **não existe, 0 I/O
aos runs**. Match → 1 I/O de verificação.

Range/seek (Alg. 3–4): LPM nos partials →
1 I/O do probe → se mismatch, reconstrói
partial (pkey_lowest / pkey_highest, como
trie) e procura de novo **só no array
in-memory**. Fig. 5: seek 010110 → probe
01001 → new_pkey 10 → terceira key.

**§3.3 Rebuild (p. 33:11–33:13).** Disco
acelera o sort-merge do compact (selectors
já ordenam). Rebuild: *compaction history*
actualiza selectors/offsets sem reler os
runs novos. Flush de run pequeno: segmentos
não afectados só actualizam cursor; affected
incorporam ou fazem split. Scan residual =
sequencial + `posix_fadvise`. Rebuild
assíncrono possível.

**§3.4–3.5 Custo e implementação (p. 33:13–33:15).**
Custo ≈ (2·keȳ + C·ρ)/π + ⌈log₂ ρ⌉/8 + P
bytes/key. Típico: ρ=8, C=4, P=4 (32-bit),
π≈64. Table 1 (workloads FB [2, 9, 19]):
Disco **<5%** do KV na maioria; **29%** no
USR (value 2 B); APP 2.36%, ETC 1.70%.
32-bit distingue ~50–400 keys (datasets
email/DBLP/dict); 16-bit 21–50; 64-bit
~1000. Layout Disco alinhado a 4 KB.
**PEXT/BMI2**. DiscoDB: partições de runs
sobrepostos + 1 Disco; recusa flush
demasiado pequeno; compact por custo
estimado.

**§4.1 Micro (p. 33:15–33:17).** Mesmo setup
da Fig. 1. Mix ~75% exist / 25% miss. Fig.
7 point: LSM explode com cache <2%; REMIX
até **4 I/O** (log₂ 32 ≈ 5 comparações de
string); Disco e B+ **≤2** em todos os
caches (Disco = 1 metadata + 1 probe).
Fig. 8 range: LSM pior (seek em cada run);
Disco ligeiramente >2 I/O no cache curto;
**metade** do REMIX; **−87.5%** vs LSM
(8 runs → ~1).

**§4.2 DiscoDB (p. 33:17–33:20).** CloudLab
c220g2, Ubuntu 22.04, 2× Xeon E5-2630, 128
GB DRAM, **Intel S3500 480 GB** Ext4.
Keys 16 B hex, values **120 B**. cgroups
16/32/64 GB. RocksDB **v8.10.0** (tuning
guide). RemixDB π=32. Disco 32-bit
partials. **4 client threads**. Load 1 B
pares ≈ **128 GB**. I/O via eBPF.

Uniform seek (Figs. 9–10): DiscoDB até
**2.3×** RemixDB e **2.8×** Rocks. 16 GB:
DiscoDB **1.45** I/O/q, Remix **3.0**,
Rocks **4.38**, Full-Index 1.8, No-Index
6.1. Throughput ∝ 1/I/O.

Point (Fig. 11): vantagem menor (Rocks usa
BF; Disco/Full-Index saltam miss).

Skew (Fig. 12, composite-Zipf 1000 hots,
θ=0.99): todos >4× vs uniforme. DiscoDB
**1.2×** Remix, **2.2×** Rocks; menos
sensível ao skew que Remix.

Optane 905P (Fig. 13): mesma ordem.

Load (Figs. 14–15): DiscoDB **−2 a −10%**
vs Remix; **1.9–2.1×** Rocks; **−50%** vs
No-Index; **2×** Full-Index. Write I/O:
Disco **+16%** vs Remix; Rocks **2.7×**
Disco; Disco **−30%** vs Full-Index. Read
I/O no load: Disco/Remix/Full-Index
**5.4×** Rocks (rebuild do índice) — sem
matar o throughput do Disco.

**§4.3 YCSB (p. 33:21–33:22).** 128 GB, 200
s, 4 threads, 16 GB cgroup. Table 2
clássica (E = seek + 50 next). DiscoDB
ganha A/B/C/F vs Remix e Rocks. D = latest
→ MemTable (Wormhole [50] nos não-Rocks,
**não** é Disco). **E: os outros perdem
para Rocks** — o scan de 50 keys domina o
I/O; o índice de seek não paga. Optane:
Remix passa Rocks (CPU > I/O); Full-Index
forte porque keys são 16 B (ganho de
compacidade do Disco some).

**§5–6 (p. 33:22–33:23).** Filtros vs
índice. Maplets [12] = hash→run, sem
range. HOT / crit-bit = bits
discriminativos in-memory; Disco é o
primeiro em LSM on-disk. Nova-LSM só L0;
Jungle LSM+COW-B+; WiscKey; Bourbon só
ints. Conclusão: 1–2 I/O; **−51% I/O vs
REMIX**; **+150–220%** throughput.

## Tese central e argumento

O read da LSM não se conserta com mais
filtro nem com compact mais agressivo. O
buraco é **índice entre runs**. REMIX
mostrou o caminho mas ainda faz log(n)
I/O de string sob cache curto. Disco
acrescenta partial keys (bits
discriminativos) para que o seek/point
foque **um** probe no run. Isso permite
*mais* runs (policy write-friendly) sem
pagar o seek-N-ways.

O paper **não** pede B+ no sítio da LSM,
nem learned index, nem range filter de
produção.

## Estrutura do texto

| § | p. | Conteúdo |
|---|----|----------|
| Abstract / 1 | 33:1 | 1 I/O; +220%; vs REMIX |
| 2 motivação | 33:3 | Fig. 1; BF sob pressão; REMIX |
| 3.1–3.2 | 33:5 | segmentos; Algs. 1–4 |
| 3.3–3.5 | 33:11 | rebuild; Table 1; PEXT |
| 4.1 micro | 33:15 | Figs. 7–8 |
| 4.2–4.3 | 33:17 | DiscoDB; YCSB Figs. 9–17 |
| 5–6 | 33:22 | related; conclusão |

## Conceitos-chave

- **Multi-run index.** Um índice sobre *todos*
  os runs; query não busca cada um (p. 33:1).
- **Segmento / âncora / run selector /
  cursor offset.** De REMIX; Disco acrescenta
  mask + partials (p. 33:5).
- **Discriminative bits / partial key.**
  Primeiro bit em que vizinhos diferem;
  extract via PEXT (p. 33:6–33:8).
- **1 I/O aos runs.** Point miss pode ser 0;
  hit/seek ≤1 probe + 1 metadata (p. 33:2,
  33:16).
- **Compaction history.** Rebuild sem reler
  os runs novos (p. 33:12).
- **DiscoDB.** LSM particionada, policy
  write-efficient *porque* o índice cobre os
  runs extra (p. 33:2, 33:15).
- **Full-Index / No-Index.** Ablations: B+
  de todas as keys vs zero índice.

## Citações relevantes

1. "filters fundamentally do not help locate items and often become the bottleneck of the system." (p. 33:1)
2. "Disco indexes all the keys in an LSM-tree, so a query does not have to search every run of the LSM-tree." (p. 33:1)
3. "Disco guarantees that both point queries and seeks issue at most one I/O to the underlying runs" (p. 33:1)
4. "improve point and range query performance by up to 220% over RocksDB" (p. 33:1)
5. "when the memory budget is limited, it requires 6× more I/Os than the B+-tree due to the cache misses when accessing Bloom filters." (p. 33:3)
6. "if the system runs on commodity hardware such as an SSD with a limited caching budget, the cost of any additional I/O quickly outstrips the sort-merge savings, so REMIX becomes ineffective." (p. 33:4)
7. "we can randomly access any key in a segment by its index using only one I/O." (p. 33:6)
8. "Disco takes less than 5% of the key-value data." (p. 33:13; Table 1, maioria)
9. "Disco and B+-tree consistently maintain less than 2 I/Os per query across all cache sizes." (p. 33:16)
10. "When compared with REMIX, Disco only uses half of the I/Os to complete a query. When compared with LSM-tree, Disco reduces the number of I/Os per query by about 87.5%." (p. 33:17)
11. "the read throughput of DiscoDB is up to 2.3× and 2.8× as much as RemixDB and RocksDB" (p. 33:18)
12. "DiscoDB uses only about 1.45 I/Os per query, whereas RemixDB and RocksDB use about 3.0 and 4.38 I/Os per query, respectively." (p. 33:18, 16 GB)
13. "the write throughput of DiscoDB is 2% to 10% lower than RemixDB" (p. 33:20)
14. "DiscoDB uses 16% more write I/O than RemixDB, whereas RocksDB write I/O is 2.7× and 3.2× to that of DiscoDB and RemixDB" (p. 33:20)
15. "DiscoDB, RemixDB, and Full-Index use 5.4× more read I/O than RocksDB" (p. 33:20, load)
16. "In workload E, the throughput of the other systems is lower than RocksDB." (p. 33:21)
17. "Disco reduces the I/O cost of queries by 51% of the state-of-the-art LSM-tree indexing techniques like REMIX." (p. 33:23)
18. "throughput improvements of 150–220% across different workloads and memory budgets." (p. 33:23)

## Diálogo teórico

- **REMIX [53] / R022** — predecessor. Disco
  = REMIX + partial keys. REMIX falha em SSD
  + cache curto (log n I/O). **Não lido.**
- **Bloom / Monkey [16] / R006** — BF não
  localiza; sob memória curta *piora*. Não
  reabre L2.
- **SuRF / Rosetta / GRF / SNARF** — range
  filter ≠ iterador; memória ≥ BF; não em
  produção (dizem).
- **Maplets [12] / Splinter** — índice
  hash→run; sem range. L14 cousin.
- **Bourbon [14] / R019** — learned, só
  ints. L15 continua REFUSE.
- **WiscKey [35] / R005** — keys no LSM
  ajuda scan; Disco indexa keys *já* no
  LSM. Complementar, não substituto.
- **R012 [43]** — citado como policy de
  compact; Disco *desacopla* read da
  policy (mais runs ok). **Não** reabre
  L37 (tiering) no Pedra.
- **Wormhole [50]** — MemTable dos
  Disco/Remix; o ganho YCSB-D **não** é
  Disco.

## Relação com Pedra / Montanha

Pedra: Bloom por SST, scan = merge
(`StreamingVisibleIter`) de mem ∪ SSTs.
`L0_COMPACTION_TRIGGER=4`,
`MAX_LSM_LEVEL=3`, compact = **Full do
par**. Poucos runs sobrepostos — não as 8
partições do DiscoDB. Prefetch de vlog
(RFC-0029) é outro eixo (value, não
índice de key). Sem PEXT, sem índice
cross-SST, sem partições.

O ganho do paper *assume* muitos runs +
cache que não cobre BF/índice. Pedra hoje
é o oposto: leveling raso, BF in-memory,
scan de 1–4 L0 + 1–2 níveis. YCSB-E
(seek+50) **perde** para Rocks — o caso
que o fold/`scan` do Caixote mais parece.

| Disco (paper) | Pedra | Ledger |
|---------------|-------|--------|
| 1 I/O/seek com 8 runs | merge de poucos SST | **L13 `MEASURE`** só se soak de `scan` mostrar I/O ≈ #runs ≥ 8 *e* cache não cobre BF. Senão o índice é custo sem benefício |
| DiscoDB + policy write-efficient | Full-do-par; L37 REFUSE | **não** inverter L37 para “justificar” Disco |
| Rebuild +5.4× read I/O no load | DST / crash mid-rebuild | hostil ao sim; precisa `Env` e atomics de índice |
| PEXT/BMI2 | portátil (macOS/ARM Caixote) | **não** dependência de ISA |
| YCSB-E pior que Rocks | fold/scan longo | **L44 `REFUSE`** Disco no kernel. REMIX (R022) continua listed até soak |
| Table 1 1.5–29% do KV | values grandes + blobs 0029 | índice de key ainda mais relativo se values saem do SST |

- **Não implementar** Disco, DiscoDB,
  partições, partial keys, PEXT, rebuild
  por compaction history. **Não** range
  filter no sítio disto. **Não** Full-Index
  (B+ de todas as keys).
- **Primeiro slice se o gate L13 abrir:**
  contar I/O de seek (quantos SST
  abertos / `table_cache_misses` por
  `scan`) no soak — sem índice novo.
- **Teste que escreveríamos se
  roubássemos a ideia:** 8 L0 sem compact
  + scan curto; se I/O/seek ≈ 8 e p99
  scan ≥10× get, *aí* um índice
  multi-run entra no menu. Até lá, L0=4
  + Full-do-par é o desenho.
- **Falsificaria L44 (não recusar Disco):**
  o soak com L=3 já tem ≥8 runs vivos *e*
  I/O/seek ≥4 *e* YCSB-E (ou scan Caixote)
  ganha >2× com um protótipo. **Falsificaria
  L13 (não medir):** o mesmo soak com
  I/O/seek ≈ 1 (já “B+”) — índice
  multi-run é no-op.

Fichas irmãs: R006 (BF sob pressão —
confirma Fig. 1); R012 (policy ≠ índice);
R013 (mais ficheiros, outro eixo);
R022 REMIX (ainda listed); R005 (KV-sep
não substitui índice).

## Avaliação crítica

- **+220% / 2.8× / 150–220%** misturam
  abstract, Fig. 9 e conclusão. O número
  auditável em SSD uniforme é **2.8×**
  Rocks no seek (Fig. 9). YCSB-E *inverte*.
- **Keys 16 B hex, values 120 B, 128 GB,
  4 threads, S3500 (SATA datacenter).**
  Keys curtas favorecem Full-Index (eles
  próprios o dizem no Optane). Pedra tem
  keys variáveis e values no vlog.
- **DiscoDB ≠ “Rocks + plugin”.** É LSM
  particionada + Wormhole + policy própria.
  Comparar DiscoDB vs Rocks mistura
  índice *e* geometria.
- **Micro = cache simulator** (LRU
  replay), não o SSD. Figs. 7–8 ≠ Figs.
  9–10.
- **Rebuild 5.4× read I/O** no load —
  escondido atrás de “writes ainda 2×
  Rocks”. Em disco mais lento ou sync
  WAL (Pedra default) o rebuild dói.
- **Não medem:** 1-writer Pedra; DST;
  ARM sem BMI2; L=3 Full-do-par; scan
  longo de fold; keys grandes / prefixo
  partilhado extremo (mask cresce).
- **REMIX não fichado.** A crítica a
  REMIX (log n I/O) é *deste* paper.

## Palavras-chave

disco; remix; multi-run-index; partial-key;
discriminative-bits; seek; range-query;
bloom-pressure; discodb; pext

## Fontes

**Primária:** `research/fontes/R023_Zhong_2025_Disco.pdf`
(PACMMOD 3(1) Art. 33, NSF PAR) + `.txt` +
`.pages.txt`.

**Secundárias (não citadas como se fossem
este paper):** `merge.rs` (scan merge);
`db.rs` (`range` / `scan` / prefetch vlog);
ficha R006; `00_estrategias.md` §1.3.
L13 listava Disco+REMIX — REMIX ainda
listed.
