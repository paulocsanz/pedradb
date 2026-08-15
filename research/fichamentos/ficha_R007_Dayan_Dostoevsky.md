# Fichamento: R007 — Dostoevsky (Lazy Leveling / Fluid LSM)

**Status:** D4
**Lido em:** 2026-08-14
**Catalog:** [CATALOG.md](../CATALOG.md) · `R007`

## Referência

DAYAN, Niv; IDREOS, Stratos. **Dostoevsky: Better Space-Time
Trade-Offs for LSM-Tree Based Key-Value Stores via Adaptive
Removal of Superfluous Merging.** In: *Proceedings of the 2018
International Conference on Management of Data (SIGMOD ’18)*,
Houston, TX, 10–15 June 2018. 16 pp. ACM.
DOI 10.1145/3183713.3196927.
URL (ACM): https://doi.org/10.1145/3183713.3196927

Páginas internas do PDF: 1–16 (corpo §§1–7 + refs + Apps A–I no
mesmo ficheiro). Locus = **p. interna do PDF**. Figs. 1, 6, 9, 10
conferidas no raster, não só no `.txt`.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract, §§1–7,
  Figs. 1–10, Table 1, Eqs. 1–14 no corpo; Apps A (Lagrange /
  Eq. 15–21), B (forma fechada de \(R\)), C (memória baixa /
  Table 2), D (recursive vs preemptive merge), E (WiscKey /
  SILT / scheduling), F (controlo experimental), G (início —
  Measurement/Computation/Transition; o resto de G + H + I
  está nas pp. 16 e seguintes do PDF de 16 pp.: Fig. 11 e o
  começo de G cabem; **H e I não têm texto extraível para
  além da Fig. 11 e da frase “experimental validation is in
  Appendix I”**). Referências [1]–[53] **não** foram relidas
  como obras.
- **Arquivo lido:** `docs/references/dostoevsky-sigmod2018.pdf` +
  extração paginada `docs/references/dostoevsky-sigmod2018.pages.txt`.
  O `.txt` sem páginas mistura Fig. 3/6/8; **não** usar como locus.
- Como foi lido: PDF local, seção a seção, nesta sessão. Fig. 10
  lida no raster (painéis A–H).
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**§1 Introduction (p. 1–2).** Mainstream LSM (LevelDB, RocksDB,
Cassandra, HBase, Accumulo, Voldemort, Dynamo, WiredTiger,
bLSM, cLSM, MyRocks, SQLite4) faz merges **igualmente caros**
em todos os níveis para (i) limitar o número de runs que um
lookup sonda e (ii) tirar entries obsoletas. O paper afirma que
os merges de *todos os níveis excepto o maior* — i.e. a maioria
— melhoram point lookup, long range e espaço por um montante
**negligível**, e pagam update (p. 1).

Assimetria (p. 2):
- **Updates:** cada nível faz o mesmo trabalho amortizado
  (merge maior = mais I/O, mas exponencialmente mais raro).
- **Point lookups (com Monkey):** FPR dos níveis rasos cai
  exponencialmente → quase todo o I/O vai ao nível \(L\).
- **Long range:** \(L\) tem a maior parte das keys do intervalo.
- **Short range:** ~1 bloco por run, independente do tamanho
  → custo **igual** em todos os níveis.
- **Space-amp:** pior caso = updates nos níveis pequenos a
  invalidar o nível \(L\).

Três peças: **Lazy Leveling** (não mergear 1…\(L-1\);
leveling só em \(L\)); **Fluid LSM-tree** (\(K\) = teto de
runs em 1…\(L-1\), \(Z\) = teto em \(L\)); **Dostoevsky**
navega \((T,K,Z)\) para maximizar throughput sob um teto de
espaço. Implementação em **RocksDB**. Fig. 1: curva abaixo
de Monkey.

**§2 Background (p. 3–4).** Recapitula LSM, \(T\), leveling vs
tiering (Fig. 2, \(T=4\)). Eq. 1 para \(L\) (p. 3). \(2\le T\le
T_\text{lim}=N/(B\cdot P)\). Regras de versão (buffer replace;
merge keep-younger; só merge com o run vizinho). Point =
primeiro hit; range = sort-merge de todos os runs. Deletes =
flag, drop no run mais velho. Fragmented merge (SSTable 2–64
MB) **não muda** o I/O worst-case, só o scheduling (p. 4).
Space-amp virou custo por causa de SSD; Facebook trocou
B-tree por leveled LSM por espaço [27] (p. 4). Fences
\(O(N/B)\), 1 I/O/run. Eq. 2 = Bloom de Tarkoma (igual a
R006). Bloom *partitioned* por bloco é “slightly higher”
FPR; offload de filtros frios é prática LevelDB (p. 4).
Aplica-se a set-membership, pointers para objects fora
(WiscKey [39]), FTL, grafos — mas a análise restringe-se
às ops básicas.

**§3 Design space (p. 4–6).** Worst-case update: todas as
writes visam keys que já estão em \(L\) (ninguém morre cedo).
Tiering: \(O(L/B)\). Leveling: \(O(T\cdot L/B)\) (média \(T/2\)
merges por nível). Fig. 3A: o trabalho **por nível** é o
mesmo. Point lookup: soma dos FPRs; SOTA uniforme
\(O(e^{-M/N}\cdot L)\) / \(O(e^{-M/N}\cdot L\cdot T)\); Monkey
tira o \(L\) (p. 5). “It is always beneficial to use Monkey”
(p. 5). Space-amp: leveling \(O(1/T)\) (Rocks \(T=10\) →
≈10% no Facebook [27]); tiering \(O(T)\) se cada run de
\(L\) tiver o mesmo set (p. 5–6, Fig. 4). Long range:
\(s/B > 2\cdot L_\text{max}\) (definição, p. 6) →
\(O(s/B)\) leveling, \(O(T\cdot s/B)\) tiering. Short range:
\(O(L)\) / \(O(T\cdot L)\). Fig. 5: leveling e tiering
particionam o espaço e encontram-se em \(T=2\); “elusive
optimal” a tracejado. Oportunidade: mergear **menos** nos
níveis pequenos.

**§4.1 Lazy Leveling (p. 6–8).** Híbrido: **tiering em
1…\(L-1\)** (até \(T-1\) runs), **leveling em \(L\)** (1 run)
(Fig. 6). Eq. 3: \(R=p_L+(T-1)\sum_{i=1}^{L-1}p_i\). Eq. 4 =
orçamento de filtros (igual à forma Monkey). Lagrange →
Eq. 5: \(p_L=R\cdot(T-1)/T\), \(p_i=R/T^{L-i+1}\) para
\(i<L\). Eq. 6: \(R=e^{-(M/N)\ln(2)^2}\cdot(\text{constante
pequena em }T)\). Complexidade point **igual** a leveling
\(O(e^{-M/N})\) *apesar* de mais runs em cima — **porque**
os Blooms são os de Monkey. Threshold \(M/N\): máximo
**1.62 bits/entry** em \(T=3\) (Eq. 7); defaults 10 ou 16
estão acima. \(V=1+R-p_L=O(1)\) (Eq. 8). Short range:
\(O(1+(L-1)\cdot T)\) — **pior** que leveling \(O(L)\).
Long range: \(O(s/B)\) — igual. Update: \(O((L+T)/B)\) —
melhor que \(O(T\cdot L/B)\). Space: \(O(1/T)\) — igual.
Fig. 7: LL domina leveling em point×update; perde em
short range; empata em long range. Em \(T=2\) tudo coincide;
em \(T\to T_\text{lim}\) vira sorted array. **“No Single
Merge Policy Rules”** (p. 8): LL para update+point+long
range; tiering se o workload é quase só write; leveling se
é quase só lookup.

**§4.2 Fluid LSM-tree (p. 8–10).** \(K\) = máx. runs em
1…\(L-1\); \(Z\) = máx. em \(L\). Active run com threshold
\(T/K\) (níveis pequenos) e \(T/Z\) (\(L\)). Specializations
(p. 8):
- \(K=1,Z=1\) → leveling
- \(K=T-1,Z=T-1\) → tiering
- \(K=T-1,Z=1\) → Lazy Leveling

Eq. 9 generaliza Monkey para \((K,Z)\). \(R=O(Z\cdot
e^{-M/N})\) (Eq. 10). Short range \(O(Z+K(L-1))\). Long
range \(O(sZ/B)\). Update
\(W=\frac{\phi}{\mu B}\bigl(\frac{T-1}{K+1}(L-1)+\frac{T-1}{Z+1}\bigr)\)
(Eq. 12). Space \(\mathrm{amp}=Z-1+1/T\) (Eq. 13). Fig. 9:
modelo, dataset **1 TB / 128 B / 4 KB / 10 bits**. Trans1:
fixar \(T\) no inflexão de LL (\(T=5\) no exemplo) e subir
\(Z\) → caminho LL→tiering. Trans2: variar \(K\) →
LL→leveling, short range quase o de leveling.

**§4.3 Dostoevsky (p. 10).**
\(\tau=\Omega^{-1}(wW+rR+vV+qQ)^{-1}\) (Eq. 14). Pesquisa
podada: só \(L_\text{max}\) valores de \(T\); \(K,Z\) convexos
→ D&C. \(O(\log^2(N/(BP))^3)\) iterações, “fraction of a
second”. Recalcula a cada **16 flushes**. Ignora \((T,K,Z)\)
que violem o teto de `amp`. App. G: Measurement /
Computation / Transition.

**§5 Evaluation (p. 10–12).** Hardware: **RAID de discos
500 GB 7200 RPM**, 32 GB DDR4, 4×2.7 GHz, Ubuntu 16.04,
ext4 **sem journal** (p. 10). Fork RocksDB: Eq. 9 nos
Blooms + listener de eventos para Fluid + Eq. 14. Buffer
2 MB, 10 bits/entry, fences 1/32 KB, **block cache = 10%
do dataset**, WAL on, direct I/O. Load default: **1 TB**,
entry 1 KB (key 128 B + value 896 B), uniforme. 3 trials.
Baselines: Rocks default (leveling \(T=10\)), Rocks
“well-tuned” (\(T\) escolhido por eles), Monkey = subset
Dostoevsky só com leveling+tiering, e 5 policies fixas
(\(T=10\)).

Fig. 10 (raster, p. 11):
- **(A)** throughput normalizado vs % point/update
  (\(10^{-2}\)–\(10^{-1}\)): Dostoevsky ≈ 1.0; Monkey
  ≈ 0.85–0.95; Well-Tuned Rocks ≈ 0.55–0.65; Default
  Rocks ≈ 0.30–0.45. Labels \(T,Z,K\) em cima (ex.
  `10,9,9` … `5,1,4` … `50,1,10`) — quase todos no
  espaço LL/Fluid, excepto os extremos.
- **(B)** nenhuma policy fixa ganha em todo o eixo:
  tiering → Trans1 → LL → Trans2 → leveling.
- **(C)** 1 / 16 / 64 TB: LL empata point com leveling
  e tem update mais baixo; o fosso cresce com \(N\).
- **(D)** update vs skew \(c\): linhas **planas**;
  leveling ~0.0055 ms, LL ~0.0025 ms (HDD).
- **(E)** 1–10 bits/entry, lookup a key existente: em
  1 bit LL ~20 ms, ainda “comparable” a leveling; em
  10 bits ambos ~10 ms.
- **(F)** space-amp vs % updates: LL e leveling sobem
  **juntos** (o claim \(O(1/T)\)).
- **(G)** range vs selectividade: LL pior no curto,
  aproxima leveling no longo; Trans2 fica no meio.
- **(H)** Dostoevsky muda \(T,Z,K\) à medida que o
  peso de short range sobe (5 updates / 1 point fixos).

**§6 Related (p. 12).** Industry: LevelDB \(T=10\) hardcoded
leveling; Rocks só leveling; bLSM força 3 níveis. Research:
fractional cascading, \(B^\varepsilon\)-tree, logs+hash, Lim FAST’16
[36] (só \(T\) em leveling), **PebblesDB** [45] (merge mais
no prefixo quente), Monkey [22]. Insight: o Bloom de Monkey
**abre** a porta para relaxar merge em baixo sem piorar
point. App. E: WiscKey compatível se o modelo contar só
keys + I/O do log; SILT/MASM (nível \(L\) unbounded) são
outra classe — update \(O(N/X)\), recusam.

**§7 Conclusion (p. 12).** Recap: mergear o mínimo para
cumprir bounds de lookup e espaço.

**Apêndices.** A: \(R=Z p_L+K\sum p_i\) (Eq. 15); \(p_{L-i}=
p_L\cdot Z/(T^i K)\). B: fecha Eq. 10. C: se \(M/N\) baixo,
fundir \(Y\) níveis sem filtro e aplicar \(Z\) também aí
(Table 2). D: preemptive merge no worst-case (sem skew).
E: WiscKey / scheduling / unbounded-\(L\). F: cada trial
num clone fresco; adaptativos pré-sintonizados pelo modelo
(isola design de transition). G: 3 fases; Fig. 11 (p. 16)
transition cost + lookup skew + model vs measure. **H e I
não extraídos.**

## Tese central e argumento

Tese (p. 1–2, 6): com Blooms estilo Monkey, point / long
range / space-amp **vivem no nível \(L\)**; o update **paga
todos os níveis por igual**. Logo os merges de 1…\(L-1\)
são “superfluous”. Lazy Leveling tira-os: update
\(O((L+T)/B)\) em vez de \(O(T\cdot L/B)\), point e space
iguais a leveling, short range pior. Fluid \((K,Z)\)
recupera o contínuo. Dostoevsky escolhe o ponto.

O argumento **depende** de Monkey (R006). Sem FPR
não-uniforme, mais runs em 1…\(L-1\) **aumentam** \(R\)
(Eq. 3). O paper admite: “It is always beneficial to use
Monkey” (p. 5) *antes* de vender LL.

Dois andares, como em R006: (i) LL = um ponto novo na
Pareto; (ii) Dostoevsky = navegador. O slogan “strictly
dominates” (p. 2, 12) é (ii), não “ponham LL no default”.

## Estrutura do texto

| § | Início (PDF) | Conteúdo |
|---|-------------:|----------|
| Abstract / Fig. 1 | p. 1 | tese + três peças |
| 1 Introduction | p. 1 | assimetria por métrica |
| 2 Background | p. 3 | LSM, Eq. 1–2, SSD space |
| 3 Design space | p. 4 | Fig. 3–5, holy grail |
| 4.1 Lazy Leveling | p. 6 | Fig. 6–7, Eqs. 3–8 |
| 4.2 Fluid | p. 8 | \(K,Z\), Fig. 8–9, Eqs. 9–13 |
| 4.3 Dostoevsky | p. 10 | Eq. 14, prune |
| 5 Evaluation | p. 10 | RocksDB, Fig. 10 |
| 6 Related | p. 12 | Monkey, Pebbles, WiscKey |
| 7 Conclusion | p. 12 | recap |
| References | p. 13 | [1]–[53] |
| Apps A–G / Fig. 11 | p. 13–16 | Lagrange, Table 2, clones |

## Conceitos-chave

- **Superfluous merging.** Merge em 1…\(L-1\) que paga
  update e quase não mexe em point/long-range/space
  (p. 1–2). Termo do paper.
- **Lazy Leveling.** Tiering em cima, leveling em \(L\)
  (\(K=T-1\), \(Z=1\)). Termo do paper.
- **Fluid LSM-tree.** Knobs \(K,Z\) + \(T\). Termo do paper.
- **Short vs long range.** Longo sse \(s/B>2 L_\text{max}\)
  (p. 6). Bloom **não** ajuda range.
- **\(amp=Z-1+1/T\).** Espaço worst-case (Eq. 13). Rocks
  \(T=10,Z=1\) → ≈10%.
- **Preemptive merge.** Incluir \(L_i\) no merge se 0…\(i-1\)
  estão cheios; óptimo no worst-case sem skew (App. D).
- **“No single merge policy rules.”** Título de secção
  (p. 8) e resultado de Fig. 10B.

## Citações relevantes

1. "merge operations from all levels of LSM-tree but the largest (i.e., most merge operations) reduce point lookup cost, long range lookup cost, and storage space by a negligible amount while significantly adding to the amortized cost of updates" (p. 1, abstract)
2. "Lazy Leveling, a new design that removes merge operations from all levels of LSM-tree but the largest" (p. 1)
3. "most point lookup I/Os target the largest level" (p. 2)
4. "It is always beneficial to use Monkey, for zero and non-zero result point lookups alike and with any kind of skew" (p. 5)
5. "in production environments using SSDs at Facebook, RocksDB uses leveling and a size ratio of 10 to bound space-amplification to ≈ 10%" (p. 5)
6. "we consider a range lookup to be long if the number of blocks accessed is at least twice as large as the maximum possible number of levels: s/B > 2 · Lmax" (p. 6)
7. "Lazy leveling at its core is a hybrid of leveling and tiering: it applies leveling at the largest level and tiering at all other levels" (p. 6)
8. "the cost complexity is O(e−M/N), the same as with leveling despite having eliminated most merge operations" (p. 7)
9. "Equation 7 has global maximum of M/N = 1.62 bits per entry (which occurs when T is set to 3)" (p. 7)
10. "Lesson: No Single Merge Policy Rules." (p. 8)
11. "K = 1 and Z = 1 give leveling. K = T−1 and Z = T−1 give tiering. K = T−1 and Z = 1 give Lazy Leveling." (p. 8)
12. "amp = Z − 1 + 1/T" (Eq. 13, p. 9)
13. "a machine with a RAID of 500GB 7200RPM disks, 32GB DDR4 main memory" (p. 10)
14. "The default setup involves inserting 1TB of data to an empty database. Every entry is 1KB (the key is 128 bytes and the attached value is 896 bytes" (p. 11)
15. "A fixed merge policy is only best for one particular proportion of updates and lookups." (p. 11)
16. "This technique is compatible with Dostoevsky but it requires a slight modification to the cost model to only account for merging keys and for accessing the log during lookups." (p. 15, App. E, WiscKey)
17. "we used cost models to find in advance the best tuning for the given workload, constructed an LSM-tree with this tuning, and then ran the experiment on it" (p. 16, App. F — os pontos “Dostoevsky” do §5 **não** medem o custo de transição)

## Diálogo teórico

No texto (não a biblio inteira):

- **Monkey [22] / R006** — pré-requisito: FPR não-uniforme
  (p. 2, 5, 12). Sem isto LL não segura o point.
- **O’Neil LSM [41]** — estrutura.
- **LevelDB [32] / RocksDB [29] / Dong CIDR’17 [27]** —
  baseline e o 10% de space-amp.
- **bLSM [48]** — 3 níveis forçados; zero-result.
- **WiscKey [39]** — App. E, compatível com adaptação.
- **PebblesDB [45]** — merge no prefixo quente (p. 12);
  outra forma de “não mergear tudo”.
- **Lim FAST’16 [36]** — só \(T\) em leveling.
- **SILT [37] / MaSM [9,10]** — \(L\) unbounded; recusam
  (update linear).
- **YCSB [21]** — invocam como “in line with”, o corpo
  **não** corre os six workloads nomeados; variam rácios
  update/lookup/range.

## Relação com Pedra

- **Crate / RFC:** `pedradb-core` `Db::compact_with_ssts_only`
  / `compact_levels`: pega o nível mais baixo com ficheiros
  e promove a `from+1` (leveling tosco).
  `MAX_LSM_LEVEL = 3`, `L0_COMPACTION_TRIGGER = 4`.
  `compact_for_reads` funde **todos** os SST num ficheiro
  em \(L_\text{max}\) (sorted-array). Sem knobs \(K,Z,T\).
  RFC-0012: Lazy Leveling **do not ship**. Bloom uniforme
  10 bits (L1 SHIP, L2 MEASURE — ficha R006).
- **Ledger L3 fica `REFUSE`, agora `src=ficha`.** O paper
  **não** diz “implementem LL como default”. Diz o
  contrário: *no single policy* (p. 8, Fig. 10B). Recusar
  como compact default do Pedra tem locus:
  1. **Depende de L2.** Eq. 3 / p. 5: sem Monkey FPR,
     \(T-1\) runs extra em 1…\(L-1\) *somam* FPR. Pedra
     ainda não tem L2.
  2. **\(L\) pequeno.** `MAX_LSM_LEVEL=3` → “most merge
     operations” não é a maioria de um \(L=7\) Rocks.
     O \(O(L)\) que LL poupa no update é 3 merges rasos.
  3. **Scan.** Short range piora para \(O(1+(L-1)T)\)
     (p. 7). Pedra tem `scan` / `scan_at` first-class
     (RFC-0015/0019). Fig. 10G/H: quando o range pesa,
     Dostoevsky *volta* a leveling / Trans2.
  4. **Hardware.** RAID 7200 RPM (p. 10). Mesmo caveat
     de R006: números de seek. Cache aqui é 10% do
     dataset (melhor que Monkey cache-off), mas o
     device continua HDD.
  5. **Navigador ≠ patch.** Fluid + retune a cada 16
     flushes + transition rewrite (App. G) é um motor
     de política. App. F admite que os pontos
     “Dostoevsky” do §5 foram **pré-sintonizados**,
     não medem adaptação online. DST-hostil.
- **Reabrir L3 só se as quatro forem verdade:** L2
  shipped; `baseline` mostra write-amp num workload
  *nomeado* com \(L\) efectivo ≥ 5; o mesmo workload
  não é short-range-bound; transition tem teste
  `FailingEnv` (crash a meio de mudar \(K,Z\)).
- **Não reabre:** L1 (Bloom por SST — o paper assume-o);
  L2 (continua MEASURE; esta ficha *reforça* que L2
  é pré-requisito de LL, não o substitui); L4/L5
  (WiscKey App. E, mesma nota que R006 p. 12).
- **Teste se um dia medíssemos LL:** mesma carga,
  `compact` atual vs “não mergear L0–L2, só L3”;
  métricas: `bytes_written_sst` / `vlog` / I/Os de
  `get` miss / `scan` de 1–10 keys. Sem false
  negative no Bloom. Sem mudar WAL.

Fichas irmãs: R006 Monkey (D4, pré-requisito); R005
WiscKey (D4, App. E); R008 PebblesDB (listed);
R012 compaction design space (listed).

## Avaliação crítica

- **HDD RAID 2018.** 1 TB em 7200 RPM. Throughput
  normalizado da Fig. 10A **não** é factor Rocks-NVMe.
  O painel D dá update em *microssegundos de disco*
  (~0.0025 vs ~0.0055 ms) — escala de seek.
- **Baseline Rocks** “default vs well-tuned” ainda é
  só leveling + \(T\). Monkey no gráfico é o **próprio
  fork** com leveling+tiering, não o binário SIGMOD’17.
  Comparação honesta *dentro* da infra RocksDB
  (eles dizem-no, p. 11).
- **App. F.** Adaptativos pré-construídos no tuning
  óptimo. “Dostoevsky dominates” no §5 **não** inclui
  o I/O de mudar de `(T,K,Z)`. Fig. 11A (p. 16) existe
  para transition cost — números dos painéis **não**
  foram lidos ponto a ponto nesta ficha.
- **1 TB / 1 KB / uniforme.** Não é o mix Facebook
  (R009). Values 896 B — se Pedra spillar para vlog
  (L4), o modelo de merge (só keys) precisa da
  adaptação que eles próprios pedem (App. E).
- **YCSB “in line with”** (p. 11) ≠ correram A–F.
- **“Strictly dominates”** (abstract / p. 2) é o
  navegador com o espaço *incluindo* LL, não LL
  sozinho. Fig. 10B mostra LL a perder nos extremos.
- **Não medem:** NVMe, compressão, multi-writer,
  Ribbon/prefix bloom, crash do retune, Pedra-class
  \(L\le 4\), DST.

## Palavras-chave

lsm; lazy-leveling; fluid-lsm; merge-policy; size-ratio;
space-amplification; leveling; tiering; monkey; rocksdb;
dostoevsky; short-range; long-range

## Fontes

**Primária:** `docs/references/dostoevsky-sigmod2018.pdf`
(SIGMOD ’18, 16 pp.) +
`docs/references/dostoevsky-sigmod2018.pages.txt`.

**Secundárias (não citadas como se fossem este paper):**
RFC-0012 companion; `db.rs` (`MAX_LSM_LEVEL`,
`compact_with_ssts_only`); fichas R005 e R006. PebblesDB
e R012 **não** foram lidos nesta ficha.
