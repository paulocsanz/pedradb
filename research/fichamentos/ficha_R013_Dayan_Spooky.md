# Fichamento: R013 — Spooky (VLDB’22)

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R013`

## Referência

DAYAN, Niv; WEISS, Tamar; DASHEVSKY, Shmuel; PAN, Michael;
BORTNIKOV, Edward; TWITTO, Moshe. **Spooky: Granulating
LSM-Tree Compactions Correctly.** *PVLDB* 15(11):3071–3084,
2022 (VLDB ’22). DOI 10.14778/3551793.3551853.
URL: https://www.vldb.org/pvldb/vol15/p3071-dayan.pdf

Locus = **p. interna do PDF** (1–14 = PVLDB 3071–3084).
Figs. 1–15 e Eqs. 1–12 conferidas no texto extraído. Fig. 13
barras: números só do parágrafo / tabela (E).

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract,
  §§1–8, Figs. 1–15, Eqs. 1–12, Algs. 1–2. Referências
  [1]–[95] **não** foram relidas. Dostoevsky [22] / Monkey
  [19] / WiscKey [58] / HashKV [15] / R010 [32] só pelo
  que *este* texto afirma (R005–R007, R010, R012, R018 já
  D4). LWC-tree [83, 84] e Endure **não** fichados.
- **Arquivo lido:** `research/fontes/R013_Dayan_2022_Spooky.pdf`
  + `.txt` + `.pages.txt`.
- Como foi lido: PDF local, seção a seção, nesta sessão.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 1–2).** Granularidade de compact ≠
eagerness (leveling/tiering). Dois campos: **Full Merge**
(Cassandra, HBase, Universal Rocks) — compacta níveis
inteiros; **Partial Merge** (LevelDB/Rocks default) —
ficheiros pequenos, pick + overlap no nível abaixo.
Falhas: Full = space-amp *transitório* ~2× no nível
maior (inputs não se apagam até acabar) → utilização
≤50%. Partial = write-amp alto por (1) *superfluous
edge merging* (ranges não coincidem) e (2) ficheiros
de vidas diferentes misturados no SSD → GC interno.
Fig. 1: Partial WA dispara com utilização; Full não
passa de 50%. **Spooky:** parte o nível \(L\) em
ficheiros iguais; parte os níveis grandes seguintes
pelas *fronteiras de \(L\)*; merge de um grupo
perfeitamente sobreposto de cada vez; Full preemptivo
nos níveis pequenos; writes/deletes sequenciais;
poucos ficheiros a escrever em simultâneo. Claim:
**>2×** menos SA que Full e **>2×** menos WA que
Partial *ao mesmo tempo*. Meta-policy: ortogonal a
eagerness, KV-sep, hardware.

**§2 Fundamentos (p. 2–3).** \(L \approx \log_T(N/B)\).
MVCC de versões de ficheiros (Rocks). **Durable SA**
= lixo até compact; **transient SA** = inputs+outputs
ao mesmo tempo. Eq. 1: total SA = 1 + transient +
durable. Eq. 2: **total WA = compact WA × GC WA**
(SSD). Leveling / tiering / lazy-leveling (Fig. 2).
**DCA** (Rocks, Fig. 3): capacidades 1…\(L-1\)
baseadas no *tamanho* de \(L\), não na capacidade;
durable SA ≤ \(1/(T-1)\) (Eq. 3). Full +
*preemption* (Fig. 4): funde vários níveis quase
cheios de uma vez. Partial + **ChooseBest** [76]
(Fig. 5): ficheiro com menos overlap no pai.

**§3 Análise (p. 4–5).** Inserções uniformes,
pior caso. Eq. 4 Full compact WA = \(L(T-1)/2\).
Eq. 5 Partial = \(L(T+1)/2\) (mais 1 ficheiro de
borda). Fig. 6: Full abaixo de Partial; Partial
*não* baixa WA ao reduzir \(T\) (borda relativa
cresce). SSD 960 GB, \(T=5\), \(B=64\) MB, DCA
on: Partial lógico **644 GB**, Full **369 GB**;
físico ~800 GB nos dois. Fig. 7A: Partial GC WA
sobe até ~3; total WA **>50** no fim do dia.
Full: GC WA ≈ 0 (um ficheiro grande de cada vez).
Eq. 6 Partial max SA = \(1 + 1/(T-1)\); Eq. 7
Full = \(2 + 1/(T-1)\). Fig. 1 \(T=10\): Full
≤50% utilização; Partial 80% com WA “exorbitant”.

**§4 Spooky (p. 5–8).** Seis princípios (Fig. 8):
(1) partir \(L\) em iguais; (2) partir alguns
níveis grandes pelas fronteiras de \(L\); (3)
*partitioned merge* de um grupo; (4) Full
preemptivo nos pequenos; (5) limitar ficheiros
abertos a escrever; (6) write/delete sequencial
por nível.

**4.1 2L-Spooky (Alg. 1).** \(L\) em ficheiros
≤ \(N_L/T\); \(L-1\) alinhado a \(L\). Níveis
0…\(L-2\): um ficheiro, Full preemptivo. Quando
\(L-1\) enche: merge par a par \(L-1\)×\(L\)
por ordem de keys. WA = Full **+1** (Eq. 8).
Transient SA = \(1/T\). Total SA = \(1 + 1/T
+ 1/(T-1)\) (Eq. 9). Fig. 9: WA quase Full;
utilização +10–30% vs Full, ~10% pior que Partial.

**4.2 Geral (Alg. 2).** Parâmetro \(X\):
dividing merge *para* o nível \(X\); partitioned
preemptivo de \(X\)…\(z\). Transient SA =
\(1/T^{L-X}\) (Eq. 11). Máx. ficheiros =
\((L-X)T^{L-X} + (X-1)\) (Eq. 12). \(X=L\) =
Full. Recomendam \(X=L-2\).

**4.3 Skew.** Não compacta ficheiros de \(L\) que
não intersectam updates novos (Fig. 11). Flush
sequencial pode criar ficheiro novo sem merge.

**4.4 Tiering / LL.** Mesmo alinhamento; \(G\)
níveis greedy leveled (impl. §5). Compatível com
LWC-tree “melhorado” no caso tiered.

**§5 Impl. (p. 9).** Um `compaction_picker` no
Rocks. rLevel 0 = extensão do buffer (\(\alpha=4\),
\(\beta=4\), \(\gamma=9\)). Mapeiam níveis lógicos
para rLevels consecutivos (tiering). Até 3
compacts em paralelo + subcompactions. L0
internal compact para não stall.

**§6 Eval (p. 9–12).** i7-11700 16 threads, 64 GB,
960 GB Samsung NVMe, ext4, Ubuntu 18.04. db_bench:
key 16 B + value 512 B. \(T=5\), memtable 64 MB,
Bloom **10 bits**, DCA on, 1 writer + 16 bg.
Spooky \(X=L-2\); Partial ficheiro 64 MB.
`fstrim` entre trials. `du` / `nvme` / stats
Rocks.

Fig. 13 (uniforme, 1 dia insert+update):
- Lógico: Partial/Spooky **644 GB**; Full **369 GB**.
- Compact WA: Spooky ~Full+um pouco (mais dados);
  **≈30%** melhor que Partial no mesmo tamanho.
- GC WA: Full ≈0; Partial pior; Spooky **>2×**
  mais barato que Partial.
- Total WA: **≈2.5×** melhor que Partial.
- Throughput uniforme (ops/s): writes Full 87300 /
  Spooky 54200 / Partial 26100; point 3860 / 2500 /
  1970; seek 489 / 316 / 233. Full “ganha” com
  **menos dados**.
- Zipf (s=1, 10 h, hotspots mudam 2/2 h): writes
  174k / **166k** / 130k — Spooky ≈ Full com 2×
  dados.
- p99.9 (F): Spooky estável (não troca throughput
  por cauda).

Fig. 14: Spooky em leveling / LL / tiering, 250 GB
lógico. \(X=L-1\) **crash** (sem espaço); \(X=L-4\)
**crash** (>1024 fds); \(X=L-2\) o equilíbrio.

Fig. 15: Partial com ficheiros 5–10 GB crash por
SA transitório; 2 GB ainda tem GC alto. “Não é
tuning, é intrínseco.”

Fig. 1 \(T=10\): Spooky ~80% utilização e **3×**
menos total WA que Partial.

**§7–8 (p. 12).** Related: LSM-bush, LSM-trie
(hash → sem range), Pebbles (partições aleatórias
grandes), LWC (fronteiras mas pode mergear *vários*
grupos no \(L\) e crashar), striped HBase. ZNS /
no-FTL / FPGA: Spooky é *picker*, compatível.
KV-sep [15, 55, 58] complementar. SILK/ADOC:
estabilidade, outro eixo. Conclusão: primeiro
granulation que junta utilização alta e WA
moderado.

## Tese central e argumento

Granularidade é um eixo **à parte** da policy
(R007/R012). Full gasta disco; Partial gasta
WA (borda + GC SSD). Spooky alinha fronteiras
ao \(L\) para merge “perfeito” e I/O sequencial
por nível. O **>2×** é *neste* NVMe quase
cheio, Rocks, \(T=5\), 16 bg threads — não um
teorema para \(L=3\) embed.

O paper **não** pede Spooky sem Partial já
existir: o picker assume ficheiros e overlap.

## Estrutura do texto

| § | p. | Conteúdo |
|---|---:|----------|
| Abstract / 1 | 1 | Fig. 1; Full vs Partial; tese |
| 2 fundamentos | 2 | SA/WA; DCA; preemption |
| 3 análise | 4 | Eqs. 4–7; Fig. 6–7; GC |
| 4.1 2L | 6 | Alg. 1; Eqs. 8–9 |
| 4.2–4.4 | 7 | \(X\); skew; LL/tiering |
| 5 impl | 9 | Rocks picker; rLevel 0 |
| 6 eval | 9 | Fig. 13–15; 644 vs 369 GB |
| 7–8 | 12 | LWC/Pebbles; ZNS; conclusão |

## Conceitos-chave

- **Full / Partial Merge.** Granularidade nível vs
  ficheiro (p. 1). Termos do paper.
- **Transient vs durable SA.** Inputs vivos +
  outputs; lixo até compact (p. 2).
- **Superfluous edge merging.** Borda não
  sobreposta reescrita (p. 4, Fig. 5).
- **ChooseBest.** Pick de menor overlap [76].
- **DCA.** Capacidades rasas = \(N_L / T^{L-i}\).
- **Preemption.** Fundir níveis quase cheios de
  uma vez (Fig. 4).
- **Dividing merge.** Compact para \(X\) partido
  pelas fronteiras de \(L\).
- **Partitioned merge.** Um grupo alinhado de
  cada vez.
- **\(X\).** Primeiro nível partido; default
  \(L-2\) (p. 11).
- **Total WA.** Compact × GC SSD (Eq. 2).

## Citações relevantes

1. "With Full Merge, space-amplification is exorbitant." (p. 1)
2. "Partial Merge exhibits excessive write-amplification." (p. 1)
3. "Spooky achieves >2x lower space-amplification than Full Merge and >2x lower write-amplification than Partial Merge at the same time." (p. 1)
4. "total write-amp = compaction write-amp × GC write-amp" (p. 3, Eq. 2)
5. "Full Merge cannot reach a storage utilization of over 50%." (p. 2)
6. "On average, one additional file’s worth of data is superfluously included in each compaction" (p. 4)
7. "total write-amp to exceed fifty by the experiment’s end" (p. 5, Partial, Fig. 7A)
8. "write-amp with 2L-Spooky = L · (T−1)/2 + 1" (p. 6, Eq. 8)
9. "The overall transient space-amp for 2L-Spooky is therefore 1/T." (p. 6)
10. "we set the parameter X, the level into which Spooky performs dividing merge operations, to L−2" (p. 10)
11. "This allows Spooky to match Partial Merge in terms of logical data size (also 644GB in this experiment)." (p. 10; Full 369 GB)
12. "Spooky improves on Partial Merge by ≈ 30% while matching it in terms of logical data size." (p. 10, compact WA)
13. "This leads to less interspersing of files with disparate lifespans within the SSD and hence > 2x cheaper garbage-collection." (p. 10)
14. "Spooky reduces total write-amp by ≈ 2.5 relative to Partial Merge" (p. 11)
15. "writes 87300 / 54200 / 26100" (p. 10, Fig. 13E uniforme: Full / Spooky / Partial)
16. "Zipfian … writes 174000 / 166000 / 130000" (p. 10, Fig. 13E)
17. "With X tuned to L−1 … the system runs out of space and crashes … With X set to L−4, the system crashes due to having too many open files" (p. 11)
18. "even with 2GB files, concurrent compactions cause these files to become physically interspersed." (p. 12)
19. "Spooky is therefore the best choice enabling excellent storage utilization and performance at the same time." (p. 12, Fig. 1 T=10: 3× menos total WA)
20. "LWC-tree does not prevent the system from merging multiple partitions at once into the largest level, and so the system may crash." (p. 12)

## Diálogo teórico

- **Dostoevsky [22] / R007** — eagerness \(K,Z\);
  Spooky é *granularity*. Aplicam Spooky a LL
  (Fig. 14) **sem** reabrir L3 como default.
- **R012 Sarkar** — 4 primitivas; Partial = LO+1
  etc. Spooky é um ponto novo no eixo
  granularity+layout, não um 11.º menu.
- **R010 Dong** — space>WA; DCA [31].
- **WiscKey [58] / HashKV [15] / R005, R018** —
  KV-sep complementar (p. 12).
- **ChooseBest [76] / Lim FAST’16 [57]** — overlap
  ≈ \(T/2\).
- **LWC-tree [83, 84]** — fronteiras sem o “um
  grupo de cada vez”.
- **PebblesDB [69]** — partição aleatória.
- **SILK [8, 9] / Luo stability [59]** — stalls;
  outro eixo.
- **ZNS [11]** — Spooky no block device; ajuda
  ZNS se um dia.

## Relação com Pedra / Montanha

Pedra (`compact_levels`): **todos** os SST de
\(N \cup N+1\) → **um** SST em \(N+1\).
`MAX_LSM_LEVEL=3`. Sem ChooseBest, sem ficheiros
alinhados, sem DCA, sem preemption de 3 níveis.
Um compact de cada vez no caminho `&mut` (bg
pipeline existe, mas não 16 threads a misturar
níveis). Transient SA = 2× o *par* que está a
fundir, **não** 2× o dataset (o Full do paper
é o nível \(L\) inteiro).

| Paper | Pedra | Ledger |
|-------|-------|--------|
| Partial + ChooseBest primeiro | não temos ficheiro-granular | **L35 MEASURE intacto.** Spooky **depois** |
| Spooky \(X=L-2\) | \(L\le 3\) → \(L-2\le 1\); quase não há “níveis grandes” a alinhar | **L11 `MEASURE` src=ficha.** Gate: (1) L35 shipped; (2) utilização SSD nomeada (nvme GC); (3) \(L\ge 4\) **ou** último nível ≫ par actual |
| Full ≤50% utilização | Full-do-par não é Full-de-\(L\) | **não** copiar o pânico dos 50%. O nosso transient é o tamanho de 2 níveis rasos |
| 16 bg + mix de vidas no SSD | writer único / poucos jobs | Eq. 2 GC WA é o *problema deles*. Medir `nvme` no nosso device antes de acreditar no 2.5× |
| Spooky + LL/tiering | L3/L37 REFUSE | **não reabre** |
| Menu / auto \(X\) | L36 | \(X\) é um knob; default \(L-2\) se L11, não um menu |

- **Não implementar** Spooky, DCA, preemption de
  4 níveis, 16 compact threads.
- **Teste se L11 (depois de L35):** soak que encha
  o NVMe; `nvme` smart-log WA; Partial (L35) vs
  picker que parte \(L\) e alinha \(L-1\). Crash
  a meio = tmp→rename já existe. Fd count <
  limite OS.
- **Falsificaria L11 (não fazer):** com \(L=3\) e
  L35, o soak não mostra GC SSD nem SA transitório
  do par. **Falsificaria “nunca”:** o mesmo soak
  com Partial (L35) a >80% do disco e total WA
  ≥2× um Spooky \(X=L-2\) no *mesmo* `Env`.

Fichas irmãs: R012 (4 primitivas; L35); R007 (LL
REFUSE); R010 (espaço); R018 (KV-sep, outro eixo).

## Avaliação crítica

- **NVMe 960 GB quase cheio, 1 dia, 16 bg.** O
  2.5× e o Fig. 1 são *deste* stress de GC. Pedra
  embed / SSD vazio não vê o produto Eq. 2.
- **Full usa 369 GB, os outros 644 GB.** Throughput
  do Full **não** é apples-to-apples (eles admitem).
- **Universal Rocks recusado** como baseline Full
  (não é leveling) — honesto, mas o Full é o
  *deles* dentro do picker Spooky.
- **Zipf s=1, hotspot muda 2 h** — não é o Zipf
  0.99 do HashKV.
- **Não medem:** Pedra \(L=3\) whole-merge; um só
  compact thread; HDD; compressão; valores
  mistos / vlog.
- **“Best choice” (p. 12)** é no espaço
  Full/Partial/Spooky no *seu* Rocks. Não é
  “implementem amanhã no Pedra”.

## Palavras-chave

spooky; compaction; granularity; full-merge;
partial-merge; space-amplification; write-amplification;
ssd-gc; dca; preemption; choosebest; rocksdb

## Fontes

**Primária:** `research/fontes/R013_Dayan_2022_Spooky.pdf`
(PVLDB 15(11), vldb.org) + `.txt` + `.pages.txt`.

**Secundárias (não citadas como se fossem este
paper):** ficha R012; RFC-0012; `db.rs`
`compact_levels`; fichas R007, R010. LWC-tree /
Endure **não** lidos.
