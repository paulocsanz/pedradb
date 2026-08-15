# Fichamento: R006 — Monkey (optimal navigable LSM)

**Status:** D4
**Lido em:** 2026-08-14
**Catalog:** [CATALOG.md](../CATALOG.md) · `R006`

## Referência

DAYAN, Niv; ATHANASSOULIS, Manos; IDREOS, Stratos. **Monkey:
Optimal Navigable Key-Value Store.** In: *Proceedings of the 2017
ACM International Conference on Management of Data (SIGMOD ’17)*,
Chicago, IL, 14–19 May 2017. p. 79–94 (ACM; DOI
10.1145/3035918.3064054). Harvard author PDF (16 pp. letter).
URL: https://stratos.seas.harvard.edu/files/stratos/files/monkeykeyvaluestore.pdf

Páginas internas do PDF: 1–16 (corpo §§1–7 + refs + apêndices A–F
no mesmo ficheiro). Locus abaixo usa **p. interna do PDF** (não a
paginação ACM 79–94). Figuras conferidas no PDF, não só no `.txt`.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract, §§1–7,
  Figs. 1–12, Table 1–2, Eqs. 1–13 no corpo; apêndices A (uso),
  B/B.1 (Lagrange + série geométrica), C (Algorithm 1–3, FPR com
  `entry` variável), D (autotune T/policy), E (baseline uniforme),
  F/Fig. 12 (block cache — **só a legenda e o claim do §5**; os
  três painéis de Fig. 12 não foram digitalizados número a número).
  Referências [1]–[35] **não** foram relidas como obras.
- **Arquivo lido:** `docs/references/monkey-sigmod2017.pdf` +
  extração paginada `docs/references/monkey-sigmod2017.pages.txt`
  (citações conferidas no `.pages.txt` e, para Eq. 3 / Fig. 6 /
  Fig. 11, no raster do PDF). O `.txt` sem páginas mistura
  fórmulas; **não** usar esse ficheiro como locus.
- Como foi lido: PDF local, seção a seção, nesta sessão. Eq. 3
  no `.txt` duplicava a linha de *tiering* nas duas ramas — o PDF
  p. 6 diz *leveling* \(R=\sum p_i\), *tiering* \(R=(T-1)\sum p_i\).
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**§1 Introduction (p. 1–2).** LSM-trees (O’Neil [27]) são o
storage layer de LevelDB, RocksDB, Cassandra, HBase, Accumulo,
Voldemort, Dynamo, WiredTiger, bLSM, cLSM, MyRocks, SQLite4
(p. 1). Updates vão a um buffer, flush vira *run* ordenado,
níveis crescem por um *size ratio* \(T\); *leveling* (1 run /
nível) vs *tiering* (\(T\) runs / nível). Point lookup usa Bloom
em RAM por run + *fence pointers* (min/max por página) para 1
I/O por run (p. 1). Fig. 1: WiredTiger / Cassandra / HBase /
RocksDB / LevelDB / cLSM / bLSM estão **fora** da curva de Pareto
lookup×update (modelos + defaults do código, não um bench único).

Dois problemas (p. 2). (1) Todos os filtros recebem o **mesmo**
bits-per-element. O custo de um zero-result lookup é a **soma**
dos FPRs; o I/O de sondar um run é \(O(1)\) graças aos fences,
independente do tamanho do run; o nível maior come o orçamento
de memória sem reduzir essa soma. (2) Os knobs (\(T\), policy,
\(M_\text{buffer}\) vs \(M_\text{filters}\)) são não-lineares e
os sistemas os deixam estáticos.

Monkey: FPR do filtro no nível \(i\) **proporcional** ao número
de entries desse run (FPR dos níveis rasos *decresce*
exponencialmente). Analiticamente tira um factor \(O(L)\) do
custo worst-case de lookup — a série geométrica dos FPRs
converge a uma constante independente de \(L\). Empiricamente,
fork de LevelDB: “50%−80%” de redução de latência nos tamanhos
que eles mediram (p. 1). O segundo eixo é um modelo fechado que
navega a Pareto (policy + \(T\) + partilha buffer/filtros).

**§2 Background (p. 3–4).** \(L=\lceil\log_T((N\cdot E/M_\text{buffer})\cdot(T-1)/T)\rceil\)
(Eq. 1, p. 3). Buffer default LevelDB = 2 MB (p. 3). Level 0 =
buffer; níveis ≥1 imutáveis. Fig. 3: *tiering* vs *leveling*
com \(T=3\), \(B\cdot P=2\). Zero-result lookup sonda **todos**
os runs; range merge-sorta os runs que overlapam (p. 3). Fences:
16 KB pages + 4-byte pointers ≈ 4 ordens de grandeza abaixo dos
dados (p. 4). Eq. 2 (Tarkoma [32]): \(\mathrm{FPR}=e^{-(\mathrm{bits}/\mathrm{entries})\ln(2)^2}\)
com \(k=(\mathrm{bits}/\mathrm{entries})\ln 2\). “All
implementations that we know of use 10 bits per entry … ≈ 1%”
(p. 4). Custos SOTA (uniforme): *tiering* lookup
\(O(L\cdot T\cdot e^{-M_\text{filters}/N})\), update \(O(L/B)\);
*leveling* lookup \(O(L\cdot e^{-M_\text{filters}/N})\), update
\(O(T\cdot L/B)\) (p. 4). Justificam focar zero-result: comuns
(insert-if-not-exist, TAO/bLSM) e são o teto de I/O inútil.

**§3 Design space (p. 4–5).** Fig. 4: o espaço LSM vai de um
**log** (*tiering*, \(T\to T_\text{lim}\)) a um **sorted array**
(*leveling*, \(T\to T_\text{lim}\)); as duas policies encontram-se
em \(T=2\). Três contendas: (C1) como fatiar \(M_\text{filters}\)
entre filtros; (C2) buffer vs filtros; (C3) \(T\) + policy sob
mistura de updates / zero-result / non-zero / ranges. SOTA:
tudo estático (p. 5). DRAM ≈ 100× o preço/bit do disco e ≈ 4×
a energia (p. 5) — daí tratar \(M\) como custo, não como “grátis”.

**§4 Monkey (p. 5–10).** Quatro knobs: \(T\), leveling/tiering,
\(p_1\ldots p_L\), \(M_\text{buffer}\) vs \(M_\text{filters}\).
Fig. 5: (a) desce a curva até a Pareto; (b) prevê o deslocar da
curva; (c) escolhe o ponto de throughput.

§4.1 (p. 6–7). Eq. 3 **no PDF** (não no `.txt`):
*leveling* \(R=\sum_i p_i\); *tiering* \(R=(T-1)\sum_i p_i\).
Eq. 4: \(M_\text{filters}\) em função dos \(\ln p_i\) pesados
pela capacidade do nível. Lagrange (App. B) → Eqs. 5–6 / Fig. 6:
\(p_i = p_1\cdot T^{i-1}\) até o FPR bater em 1 nos níveis
fundos (`L_unfiltered`); aí o filtro **desaparece**. Intuição:
mais bits-per-entry nos runs **pequenos**.

§4.2 (p. 7–8). Eq. 7: \(R=R_\text{filtered}+R_\text{unfiltered}\).
Eq. 8: \(M_\text{threshold}=N\cdot\ln(T)/((T-1)\ln(2)^2)\); abaixo
disso o último nível perde o filtro. Non-zero: \(V=R-p_L+1\)
(Eq. 9). Update \(W\) (Eq. 10) inclui \(\phi=\) custo write/read.
Range \(Q=sN/B+\) (#runs) (Eq. 11). Fig. 7: modelo com
\(N=2^{35}\), \(E=16\) B, \(T=4\), buffer 2 MB, \(M_\text{filters}\)
0–35 GB, **512 TB** de dados — Monkey abaixo do SOTA para todo
\(M_\text{filters}\). V, W, Q são **iguais** ao SOTA: Monkey não
muda merge nem range.

§4.3 (p. 8–9) + **Table 1**. Com \(M_\text{filters}/N>1.44\)
(teto em \(T=2\); “typically 10, far above 1.44”, p. 8) o custo
Monkey é \(O(e^{-M_\text{filters}/N})\) (*leveling*) —
**independente de \(L\) e do buffer**. SOTA continua
\(O(L\cdot e^{-M_\text{filters}/N})\). Fig. 8/9: “sweet spot”
= buffer o maior possível *antes* de matar os filtros; §4.4
aproxima 5% buffer / 95% filtros depois de um piso
\(M_\text{threshold}/T^L\). Threshold prático de \(R\):
\(\sim 10^{-4}\) em HDD (seek ~10 ms) vs \(\sim 10^{-2}\) em
flash (p. 10).

§4.4 (p. 9–10). Throughput \(\tau=1/(\theta\cdot\Omega)\),
\(\theta=rR+vV+qQ+wW\) (Eqs. 12–13). Divide-and-conquer em
\(O(\log T_\text{lim})\) sobre o contínuo tiering↔leveling
(Fig. 10, App. D). SLA = recortar a caixa (R ou W máximos).

**§5 Experiments (p. 10–12).** Hardware: **500 GB 7200 RPM
HDD**, 32 GB DDR4, 4×2.7 GHz, Ubuntu 16.04, ext4 **sem
journal** (p. 10). Fork de LevelDB + tiering + \(T\) variável
+ Algorithm 1 (App. C). Default do *bench*: \(T=2\), buffer
**1 MB**, **5 bits/entry** no orçamento global (Monkey só
reparte), direct I/O, **block cache OFF** no corpo (pior caso;
App. F / Fig. 12 diz que a vantagem sobrevive com cache).
Load: 1 GB, entries 1 KB, uniforme, depois 16 K zero-result;
3 trials, error bar = 1 σ.

Fig. 11 (p. 11, lida no raster):
- **(A)** \(N=2^{15}\ldots 2^{23}\): LevelDB sobe ~2.5 → ~8 ms;
  Monkey fica ~1.5 ms (~0.2 I/Os/lookup). Texto: “up to 80%”
  (p. 10).
- **(B)** entry 32–4096 B, \(N\) fixo: o mesmo padrão.
- **(C)** 0–10 bits/entry: em 0 coincidem (~55 ms); Monkey
  desce mais cedo; “up to ≈ 60% smaller” memory para empatar
  LevelDB (p. 11).
- **(D)** non-zero + coeficiente de temporalidade: LevelDB
  ~11 ms, Monkey ~8 ms; “up to 30%” (p. 11); ~1 I/O obrigatório
  (o hit). Quase insensível à localidade — o nível grande ainda
  é a maior parte das keys.
- **(E)** Pareto lookup×update: Monkey desloca a curva para
  baixo; “up to 60%” lookup e, trocando \(T\)/policy, “up to
  70%” update (p. 11–12).
- **(F)** Navigable Monkey (escolhe T2/T4/L4/L6/L8/L16):
  “more than doubles throughput relative to LevelDB” (p. 12).
  Curva em sino: extremos (quase só writes ou só lookups)
  permitem um ponto mais agressivo.

**§6 Related (p. 12).** SOTA uniforme [4, 12, 15, 18, 19, 21,
29, 34]. WiredTiger default **16** bits/element, \(T\) a
partir de 15. LSM-trie só *tiered*. Lim FAST’16 [23] modela
update com skew só em *leveling* — Monkey modela lookup+update
nas duas policies. WiscKey [26]: “compatible with Monkey’s
core design, but it would require adapting the cost models”
(só keys no merge; lookup vai ao log) (p. 12). VT-tree /
trivial-move de SST sem overlap: já no fork LevelDB. Merge
scheduling / dedicated compact servers: ortogonal. Redis /
Memcached: fora de escopo.

**§7 Conclusion (p. 12).** Recapitula Pareto + modelo. Sem
número novo.

**Apêndices (p. 13–16).** A: qualquer LSM pode só roubar a
alocação de FPR. B: \(p_{L-i}=p_L/T^i\); \(R\) vira série
geométrica (Eq. 14). Recomendam Eqs. 17–18 na prática (não
as 5–6 simplificadas) quando \(L\lesssim 5\). C: Algorithm 1
rouba \(\Delta\) bits entre pares de runs até minimizar
\(\sum\mathrm{FPR}\); precisa de `runs[i].entries` como
metadata. D: binary search no contínuo. E: SOTA
\(p_i=R/L\) (*leveling*). F / Fig. 12: 0 / 20% / 40% cache —
claim: a vantagem mantém-se; **números dos painéis não
extraídos**.

## Tese central e argumento

Tese (p. 1–2, 6): o I/O esperado de um point lookup
zero-result **é a soma dos FPRs**. Bits uniformes **não**
minimizam essa soma, porque o custo de um I/O é o mesmo em
qualquer run (fences) e o nível fundo é exponencialmente mais
caro de filtrar. Solução: \(p_i \propto\) entries(\(i\)), i.e.
\(p_i = p_1\cdot T^{i-1}\) até \(p=1\). A série converge →
lookup \(O(1)\) em \(L\) quando há ≳ 1.44 bits/entry.

O argumento tem dois andares que o abstract mistura: (i)
alocação de Bloom — vertical shift da curva (Fig. 5a, Fig. 11
A–E, “Fixed Monkey”); (ii) navegação \(T\)+policy — movimento
*ao longo* da curva (Fig. 5c, Fig. 11F, “Navigable Monkey”).
(i) é um patch local num LSM já nivelado. (ii) é um sintonizador
de compact.

## Estrutura do texto

| § | Início (PDF) | Conteúdo |
|---|-------------:|----------|
| Abstract / Fig. 1 | p. 1 | tese + 50–80% + Pareto cartoon |
| 1 Introduction | p. 1 | SOTA, dois problemas, contribuições |
| 2 Background | p. 3 | LSM, Eq. 1–2, fences, custos \(O(\cdot)\) |
| 3 Design space | p. 4 | Fig. 4, C1–C3 |
| 4 Monkey | p. 5 | knobs, 4.1 FPR, 4.2 modelos, 4.3 \(O(L)\), 4.4 \(\tau\) |
| 5 Experiments | p. 10 | HDD, LevelDB, Fig. 11 |
| 6 Related | p. 12 | WiscKey, LSM-trie, Lim, VT-tree |
| 7 Conclusion | p. 12 | recap |
| References | p. 13 | [1]–[35] |
| App. A–F | p. 13–16 | Lagrange, Algorithms 1–5, baseline, cache |

## Conceitos-chave

- **Soma dos FPRs = \(R\).** Não o FPR médio. Um filtro 8% no
  L0 e um 8% no último nível custam o mesmo I/O esperado
  (Eq. 3, p. 6). Termo do paper.
- **FPR \(\propto\) entries do run.** Fig. 6 / “\(T\) times
  higher” (p. 6). Termo do paper. *Não* é “mais bits no nível
  fundo” — é o contrário.
- **`L_unfiltered`.** Quando o orçamento não dá, o nível fundo
  fica sem filtro (\(p=1\)) (Fig. 6, p. 6).
- **Fence pointers.** Array min/max por página em RAM; 1 I/O
  por run (p. 3–4). Ancestral: LSM 1996 era B-tree por run
  [27]. Pedra: sparse index + bounds no SST — o mesmo papel.
- **Leveling vs tiering.** 1 run vs até \(T\) runs / nível
  (p. 3). Contínuo no mesmo eixo quando \(T\) varia (Fig. 4).
- **\(M_\text{threshold}\).** Bits abaixo dos quais \(p_L\to 1\);
  1.44 bits/entry no pior \(T=2\) (p. 8).
- **Fixed vs Navigable Monkey.** Só FPR vs FPR+\(T\)+policy
  (p. 12, Fig. 11F). Distinção do próprio paper.
- **Zero-result vs non-zero.** \(R\) vs \(V=R-p_L+1\) (Eq. 9).
  O 80% é sobretudo zero-result; non-zero é ~30% (p. 11).

## Citações relevantes

1. "worst-case lookup cost is proportional to the sum of the false positive rates of the Bloom filters across all levels" (p. 1, abstract)
2. "Monkey reduces lookup latency by an increasing margin as the data volume grows (50%−80% for the data sizes we experimented with)" (p. 1, abstract)
3. "setting the false positive rate of each Bloom filter to be proportional to the number of entries in the run that it corresponds to (meaning that the false positive rates for shallower levels are exponentially decreasing)" (p. 2)
4. "this way shaves a factor of O(L) from the worst-case lookup cost" (p. 2)
5. "All implementations that we know of use 10 bits per entry for their Bloom filters by default" (p. 4); "The corresponding false positive rate is ≈ 1%" (p. 4)
6. "With leveling every level has at most one run and so R is simply equal to the sum of FPRs across all levels." (p. 6, prosa da Eq. 3; PDF confirma \(R=\sum p_i\), não \((T-1)\sum p_i\))
7. "the optimal FPR at Level i is T times higher than the optimal FPR at Level i − 1" (p. 6)
8. "It is therefore better to set relatively more bits per entry (i.e., a lower FPR) to the filters at smaller levels." (p. 7)
9. "lookup cost in Monkey is independent of the LSM-tree’s buffer size" (Table 1 caption, p. 8)
10. "the number of bits-per-element is typically 10, far above 1.44, and so for most practical purposes the complexity of Monkey is O(Rfiltered)" (p. 8)
11. "a machine with a 500GB 7200RPM disk, 32GB DDR4 main memory" (p. 10)
12. "the overall amount of main memory allocated to all Bloom filters is Mfilters/N = 5 bits per element" (p. 10); "all reported experiments in the main part of the paper are set with the block cache of LevelDB and Monkey disabled" (p. 10)
13. "Monkey dominates LevelDB by up to 80%, and its margin of improvement increases as the number of entries grows" (p. 10, Fig. 11A)
14. "Monkey can match the performance of LevelDB with a smaller memory footprint (up to ≈ 60% smaller in this experiment" (p. 11, Fig. 11C)
15. "Monkey improves lookup latency by up to 30% in this experiment for non-zero-result lookups" (p. 11, Fig. 11D)
16. "Monkey improves lookup cost by up to 60%, and this gain can be traded for an improvement of up to 70% in update cost" (p. 11–12, Fig. 11E)
17. "Navigable Monkey more than doubles throughput relative to LevelDB" (p. 12, Fig. 11F)
18. "This technique is compatible with Monkey’s core design, but it would require adapting the cost models to account for (1) only merging keys, and (2) having to access the log during lookups." (p. 12, sobre WiscKey [26])

## Diálogo teórico

No texto (não a biblio inteira):

- **O’Neil LSM [27]** — estrutura que Monkey não abandona (§2).
- **LevelDB [19]** — fork e baseline de *todos* os números do §5;
  \(T=10\) hardcoded, só leveling, FPR uniforme.
- **RocksDB [15] / Dong CIDR’17 [14]** — SOTA uniforme; MyRocks
  como prova de que SQL também senta em LSM (p. 1).
- **bLSM [29] / cLSM [18]** — Fig. 1; bLSM também para
  insert-if-not-exist como motivo de zero-result (p. 4).
- **Bloom [10] + Tarkoma [32]** — Eq. 2.
- **Kuszmaul [20]** — leveling vs tiering.
- **Athanassoulis RUM [8, 7]** — o triângulo lookup/update/\(M\)
  é o enquadramento (p. 1–2).
- **WiscKey [26]** — compatível; modelos *não* se aplicam crús
  (p. 12). Ficha irmã R005.
- **Lim FAST’16 [23]** — update com skew só em leveling; Monkey
  diz ir além (p. 12).
- **LSM-trie [35]** — só tiered, bits fixos.
- **WiredTiger [34]** — \(T\) dinâmico a partir de 15, **16**
  bits/element uniformes (p. 12).
- **VT-tree [30]** — skip de runs sem overlap; já no LevelDB.
- **LinkBench / TAO [6, 11]** — point lookups “common”.

## Relação com Pedra

- **Crate / RFC:** `pedradb-core` `bloom.rs` + `sst/table.rs`
  (`DEFAULT_BITS_PER_KEY = 10`, `rebuild_bloom` usa esse valor
  para *todos* os SST). RFC-0014 P0.1 (Bloom shipped).
  RFC-0012 companion: alocação Monkey **não** shipped.
  `MAX_LSM_LEVEL = 3` (`db.rs`) → no máximo 4 níveis (0…3).
- **Ledger:**
  - **L1 `SHIP` confirmado.** O paper assume um filtro **por
    run** com fences; Pedra já tem Bloom on-disk por SST +
    bounds/sparse index. Isso é o SOTA que Monkey *parte de*,
    não o que Monkey *inventa*.
  - **L2 fica `MEASURE`, agora com ficha.** A tese (soma dos
    FPRs, \(p_i\propto n_i\)) está no PDF. **Não promover a
    `SHIP`.** Razões que *esta* leitura deu, não folklore:
    (1) o 50–80% é em **HDD 7200 RPM**, cache off, zero-result,
    \(T=2\), 5 bits/entry — Pedra vive em SSD/NVMe e o próprio
    paper baixa o alvo de \(R\) de \(10^{-4}\) para \(10^{-2}\)
    em flash (p. 10); (2) \(O(L)\) só dói quando \(L\) cresce;
    com `MAX_LSM_LEVEL=3` o factor é ≤ 4, e Fig. 11A só abre o
    fosso depois de \(2^{20}\) entries; (3) Pedra já está nos
    10 bits/entry (acima de 1.44) — o regime em que Monkey é
    \(O(1)\) vs SOTA \(O(L)\), mas \(L\) pequeno anula o
    slogan; (4) Navigable Monkey (\(T\)+tiering) é *outro*
    produto — compact Pedra é leveling + count/bytes; RFC-0012
    já recusou Lazy Leveling (R007) sem bench.
- **Como implementaríamos L2 se o MEASURE fechar:** não um
  `with_monkey_bloom(true)`. `rebuild_bloom` / writer v3+
  recebe `bits_per_key` por SST a partir do *entry count do
  run* (Algorithm 1, App. C) ou, mais barato, da *capacidade
  do nível* (Eqs. 17–18; o paper recomenda estas quando
  \(L\lesssim 5\) — o nosso caso). Metadata já temos: `t.len()`.
  Teste: mesma carga, comparar I/Os de `get` miss (chave
  ausente) com bits globais iguais; o allocator não pode
  inventar false negative.
- **O que falsifica o claim no Pedra:** (1) 1–2 níveis ocupados
  (soma de 1–2 FPRs ≈ FPR uniforme); (2) block cache quente
  (o 80% do corpo é cache-off; Fig. 12 só afirma “maintains
  advantages”, sem número nesta ficha); (3) workload só
  non-zero (eles medem 30%, não 80%); (4) values no vlog
  (R005): o modelo de \(R\) conta 1 I/O por run da LSM, não
  o I/O extra do blob (eles próprios pedem adaptar, p. 12);
  (5) comparar contra Rocks 2024 com Ribbon / prefix bloom,
  não contra LevelDB 2017 em HDD.
- **Compatível com o que já recusámos / shipped:** WiscKey
  spill (L4) *não* bloqueia L2 — o paper diz compatível.
  Não reabre L5b (WAL) nem L3 (Lazy Leveling).

Fichas irmãs: R005 WiscKey (D4; p. 12 desta); R007 Dostoevsky
(D4 — L3 REFUSE; LL *exige* esta alocação); R010 Rocks Experience
(workload real de FPR); R004 bLSM.

## Avaliação crítica

- **Hardware 2017:** um **HDD** 7200 RPM. Os 50–80% e o “0.2
  I/Os ≈ 1.5 ms” são seek-bound. Em NVMe o mesmo 0.2 I/O é
  dezenas de µs e o paper já diz que o alvo útil de \(R\) sobe
  duas ordens (p. 10). Transportar o abstract para Pedra sem
  esse hedge é **SIGNIFICATIVO**.
- **Baseline:** LevelDB, \(T=2\) (não o default 10 do LevelDB
  de produção), 5 bits/entry (não os 10 que o §2 chama de
  universal), cache off, 1 KB values, uniforme, ext4 sem
  journal. “Fixed Monkey” só muda a alocação — honesto.
  “Navigable Monkey” muda \(T\) *e* policy: o 2× de
  throughput **não** é o ganho do Bloom.
- **Fig. 1** é modelo + defaults de código, não o mesmo bench
  do §5. Não citar Fig. 1 como medição.
- **Fig. 7** é 512 TB *simulado* pelo modelo, não um run de
  512 TB.
- **Range:** \(Q\) é o mesmo para Monkey e SOTA (Eq. 11). Bloom
  não ajuda scan. Quem citar Monkey para `scan` está a
  inventar.
- **App. C vs Eqs. 5–6:** com entry size variável o papel
  *não* usa a fórmula fechada — usa o greedy de bits. Pedra
  tem values de tamanhos mistos (e vlog). Se L2 fechar, o
  caminho honesto é Algorithm 1 (ou Eqs. 17–18 por *n*
  medido), não copiar \(p_i=p_1 T^{i-1}\) assumindo \(E\)
  constante.
- **Não medem:** compressão, multi-writer, prefix bloom /
  Ribbon, Rocks 5.x+, NVMe, key-size mix do Facebook (R009),
  crash do retune de filtros, DST.
- **“without losing anything”** (p. 12, related) é contra
  FPR uniforme no mesmo orçamento, não contra espaço ou
  CPU do filtro. Hedge do paper no §4: filtros no nível
  fundo *podem* ir embora (\(p=1\)) se \(M\) for curto.

## Palavras-chave

lsm; bloom-filter; false-positive-rate; fence-pointers;
leveling; tiering; size-ratio; pareto; lookup-amplification;
memory-allocation; leveldb; monkey

## Fontes

**Primária:** `docs/references/monkey-sigmod2017.pdf` (SIGMOD ’17,
PDF Harvard, 16 pp.) + `docs/references/monkey-sigmod2017.pages.txt`.

**Secundárias (não citadas como se fossem este paper):**
RFC-0014 P0.1; RFC-0012 companion; `crates/pedradb-core/src/bloom.rs`
(`DEFAULT_BITS_PER_KEY = 10`); ficha R005 (WiscKey, para o
parágrafo da p. 12). Dostoevsky **não** foi lido nesta ficha.
