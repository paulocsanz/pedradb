# Fichamento: R031 — Lethe (SIGMOD’20)

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R031`

## Referência

SARKAR, Subhadeep; PAPON, Tarikul Islam; STARATZIS,
Dimitris; ATHANASSOULIS, Manos. **Lethe: A Tunable
Delete-Aware LSM Engine.** In: *Proceedings of the 2020
ACM SIGMOD International Conference on Management
of Data (SIGMOD ’20)*, 14–19 June 2020, Portland, OR.
pp. 893–908.
DOI: 10.1145/3318464.3389757
URL (autor): https://cs-people.bu.edu/mathan/publications/sigmod20-sarkar.pdf
ACM: https://dl.acm.org/doi/10.1145/3318464.3389757
arXiv 2006.04777 = “updated version” (**não** lido).
Lethe+ TODS’23 [disc.bu.edu/papers/tods2023-sarkar]
**não** lido.

Locus = **p. SIGMOD** (893–908 = PDF 1–16). Figs. 1–6 e
Tables 1–2 conferidos no texto extraído. Barras da Fig. 6:
números só do parágrafo. Abstract 1.17–1.4× / 2.1–9.8× /
4–25% WA **não** reaparecem com dispositivo em §5 —
§5.1 auditável = +17% lookup, ~48% space (Dth=50%),
WA inicial 1.4× → fim +0.7%.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract,
  §§1–7, Figs. 1–6, Tables 1–2, Eqs. 1–3. Referências
  [1]–[70] **não** foram relidas. Monkey [21] /
  Dostoevsky [23] / RUM [13] / X-Engine [39] /
  Rocks DeleteRange [58] só pelo que *este* texto
  afirma (R006, R007, R012 já D4). Callaghan blog
  [15] e GDPR blog [26] **não** lidos. Lethe+ TODS
  e arXiv **não**.
- **Arquivo lido:** `research/fontes/R031_Sarkar_2020_Lethe.pdf`
  + `.txt` + `.pages.txt`.
- Como foi lido: PDF local (cópia do autor BU, acmart
  SIGMOD’20, 16 p.), seção a seção, nesta sessão.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 893–895).** LSM trata delete como
cidadão de segunda: tombstone sem garantia de quando
chega ao último nível; range delete só na sort key.
Delete persistente rápido sem matar o read é o que
streaming (janela), GDPR/CCPA (right-to-be-forgotten)
e cloud (espaço) pedem. Latência de persistência
**potencialmente ilimitada**; o escape hatch da
indústria é *full-tree compaction* (X-Engine: 1/7 ou
1/30 do DB por dia; “I/O utilization often peaks”).

Dois cenários: (1) EComp — deletes no *sort key*
(point/range em `order_id`); (2) DComp — *secondary
range delete* (sort = `document_id`, delete =
timestamp). SoA falha nos dois: espaço, read, WA,
privacidade; e full-tree para o segundo.

Lethe = **FADE** + **KiWi**. FADE: compact
delete-aware (contagem de inválidos, idade do
tombstone mais velho, overlap). KiWi: *delete tiles*
— tile ordenado na delete key, página internamente
na sort key; range secundário *dropa páginas*
inteiras. Abstract: read **1.17–1.4×**, space
**2.1–9.8×** menor, WA **+4–25%**. Contribuições
p. 895: 1.4× read “atribuído” a space até 9.8× com
só 10% deletes.

**§2 Fundo (p. 895–896).** Notação Dayan: L níveis,
T size ratio, Level 0 = buffer (neste paper), 1…L−1
disco. Leveling vs tiering vs híbrido [23, 24].
Partial compaction por ficheiro (overlap mínimo =
WA). Point lookup: BF por ficheiro + fence por
página.

**§3 Impacto dos deletes (p. 896–898).**

*Primary.* Point tombstone no run; só se descarta
no compact com o **último** nível. Range tombstone
em bloco à parte [29]; histogram em memória em
*cada* point query [15, 58]. Persistência = L
compacts, um por nível; ritmo = inserções *únicas*.
Workloads adversários (hot updates nos níveis de
cima; insert+delete recente) **reciclam** o
tombstone e nunca o empurram.

*Secondary.* “delete older than D days”: entradas
espalhadas; SoA = full-tree, custo O(N/B)
independente da selectividade.

*Limite Rocks:* pick pelo nº de tombstones [29]
reduz inválidos, **não** dá Dth.

Table 1 (valores de referência): N=2^20, T=10,
L=3 disco, M=16 MB, E=1 KB, λ=0.1, I=1024
entries/s, h=16.

*Modelos.* Space sem delete: O(1/T) leveling /
O(T) tiering. Com delete e λ pequeno: pior —
poucos bytes de tombstone invalidam muitos de KV.
Read: tombstones no BF sobem FPR; range varre
lixo (0.5% de 100 GB = 500 MB). WA: O(L·T)
leveling / O(L) tiering (inválidos reescritos).
Latência de persistência: leveling
O(T^{L−1}·P·B / I) — “remarkably high” em
update-intensive alto.

**§4.1 FADE (p. 898–900).** Família de policies
com Dth (SLA de retenção). TTL por nível
*exponencial*: d_i = T·d_{i−1},
d_0 = Dth·(T−1)/(T^L − 1), Σ d_i = Dth.
Uniforme Dth/L esgota TTL em massa nos níveis
grandes. Recalcula d_i a cada flush se a árvore
crescer.

Metadados por ficheiro: **a_max** (idade do
tombstone mais velho) e **b** (pf + rd_f
estimado por histogramas). Rocks já tem seqnum,
num_entries, num_deletes → “**no metadata
footprint**” (só 8 B de timestamp por ficheiro).

Trigger: saturação **ou** TTL expirado. Pick:

| Modo | Trigger | Pick | Optimiza |
|------|---------|------|----------|
| SO | saturação | overlap mín. | WA |
| SD | saturação | maior b | space |
| DD | TTL | tombstone expirado | Dth |

Empate: tombstone mais velho (SD/DD); mais
tombstones (SO); nível mais baixo (stall).

Implicações: samp volta a O(1/T) / O(T) *com*
deletes. WA: pico inicial, depois amortiza
(menos lixo nos compactos seguintes). Read:
menos entradas no BF → FPR ↓; se Nδ cabe em
Lδ < L, menos níveis. WAL: se o purge do WAL
< Dth, ok; senão rotina que copia live records.

Dth prático: full-tree a cada **7 / 30 / 60**
dias [39]. *Blind deletes:* FADE só insere
tombstone se o BF der positivo.

**§4.2 KiWi (p. 900–902).** Camada nova: nível →
ficheiro (ordenado em S) → **delete tile**
(ordenado em S entre tiles; páginas do tile
ordenadas em D) → página (entradas em S).
Range secundário = *full page drop* (sem ler) +
*partial* nas bordas (≤1 página/tile). h =
páginas/tile; **h=1 = layout clássico**. BF **por
página** (mesmo FPR, sem reconstruir no drop).
Fence S por tile + fence D por página. Overhead
de memória: se |S|=|D|, um sort key por tile.

CPU: L·h/4 hashes a mais em hit, L·h em miss.
MurmurHash 80 ns vs SSD 100 µs. Short range:
O(h·L). Long range: assimptótico igual. Eq. 1–3:
escolhe h pela razão lookups / secondary range
deletes. Ex.: 400 GB, 4 KB, 50 M points + 10 K
short ranges entre dois range-deletes, FPR≈0.02,
T=10 → h ≲ 10^2.

**§4.3 Lethe (p. 902–903).** Knobs: Dth (SLA) e
h (Eq. 3). Implementação: RocksDB (leveling,
L1 tiered, T=10). API de trigger/pick custom.
age = único extra por ficheiro.

**§5 Eval (p. 903–905).** 2× Xeon Gold 6230, 384
GB RAM, **240 GB SSD**. Default: DB vazia,
I=2^{10} entries/s, E=1 KB, buffer **1 MB**
skiplist, T=10, 10 bits/key, compact > write,
**block cache + direct I/O, WAL off**. Deletes
só em keys existentes, uniforme. Ingest **1 GB**.
Dth = **16.67 / 25 / 50%** do runtime (2 / 3 /
6 meses dum ano). Lookups *depois* de popular.
Workload: YCSB-A (50/50) + deletes **2–10%**
do ingest. Sem delete-benchmark padrão.

Fig. 6 (A–G) FADE vs Rocks:

- Sem deletes: idênticos (SO = SoA).
- Space: Dth=50% já **~−48%**; Dth menor, mais.
- Compactos: **−45%** jobs com 2% deletes; **+4.5%**
  bytes por job (Dth=50%) — menos ficheiros, mais
  overlap.
- Lookup: até **+17%** (BF mais limpo; miss em
  key já persistida sem I/O de tombstone).
- Idade: Lethe persiste **40–80%** mais
  tombstones; Rocks com Dth=50% deixa
  **~40 000** tombstones (**~40%** dos inseridos)
  em ficheiros *mais velhos* que Dth; Lethe
  **todos** dentro.
- WA: 1.º snapshot **1.4×** bytes; fim **+0.7%**
  (Dth=60 s, snapshots 180 s — “worst case”
  15× mais curto que o run).
- Escala: write latency **+0.1–3%**; mix
  **−0.5–4%**.

Fig. 6 (H–L) KiWi: mais selectividade + h
pequeno → menos full drops. Lookup I/O **linear
em h**. Selectividade 0.01% → h=1 óptimo;
0.05% → h=64. 90 GB / 24 h, um range delete
de **1/7** do DB: h*=8, I/O **−76%** vs Rocks,
hash **5×** (escondido atrás do disco).
Correlação S–D ≈ 1 → h=1 (KiWi no-op).

**§6–7 (p. 905–906).** Bulk delete relacional;
Vanish / “forgetting”. Conclusão: SoA é
subóptimo *já com poucos deletes*; persistência
ilimitada; FADE dá Dth com read↑ space↓ e WA
“modesto”; KiWi dá range secundário afinável.

## Tese central e argumento

Delete persistente é um **objectivo de desenho**
à parte do triângulo RUM. Sem Dth o tombstone
pode reciclar-se para sempre; o full-tree da
indústria compra o Dth com spike de I/O. FADE
rola o mesmo trabalho por TTL exponencial +
pick por idade/densidade, usando metadata que
o Rocks já tem. KiWi é *outro* eixo: layout
tecido S×D para drop de página, não um compact
melhor. Os dois juntos = Lethe; um não implica
o outro.

O paper **não** pede tiering, 16 threads, nem
desligar o WAL (o eval é que o desliga).

## Estrutura do texto

| § | p. | Conteúdo |
|---|---:|----------|
| Abstract / 1 | 893 | cidadão 2.ª; FADE+KiWi; 1.17–1.4× |
| 2 fundo | 895 | T, L, partial, BF |
| 3 impacto | 896 | primary/secondary; modelos; Table 1 |
| 4.1 FADE | 898 | TTL; a_max/b; SO/SD/DD |
| 4.2–4.3 KiWi / Lethe | 900 | tiles; Eqs. 1–3; Rocks |
| 5.1–5.2 | 903 | Fig. 6 A–G / H–L |
| 6–7 | 905 | related; conclusão |

## Conceitos-chave

- **Delete persistence latency.** Tempo entre o
  tombstone entrar e o compact com o último
  nível (p. 894). Termo do paper.
- **Dth.** Limiar de SLA; FADE garante
  a_max < Dth (p. 898–900).
- **FADE.** Família de trigger+pick
  delete-aware (SO/SD/DD).
- **a_max / b.** Idade do tombstone mais velho;
  inválidos estimados (point + range via
  histograma).
- **TTL exponencial.** d_i ∝ T^i para expirar
  a taxa constante (p. 899).
- **KiWi / delete tile.** h páginas ordenadas
  em D; página em S. h=1 = clássico.
- **Full / partial page drop.** Range
  secundário sem full-tree (p. 901).
- **Blind delete.** Tombstone contra key
  inexistente; FADE filtra com BF (p. 900).
- **Secondary range delete.** Delete key ≠
  sort key (p. 897).

## Citações relevantes

1. "State-of-the-art LSM engines do not provide guarantees as to how fast a tombstone will propagate to persist the deletion." (p. 893)
2. "LSM-trees have a potentially unbounded delete persistence latency." (p. 894)
3. "Forcing compactions to set a delete latency threshold, leads to significant increase in compaction frequency, and the observed I/O utilization often peaks." (p. 894, X-Engine)
4. "higher read throughput (1.17− 1.4×) and lower space amplification (2.1− 9.8×), with a modest increase in write amplification (between 4% and 25%)." (p. 893)
5. "A tombstone is discarded during its compaction with the last level of the tree, making the logical delete persistent." (p. 896)
6. "This reduces the amount of invalid entries, but it does not offer persistent delete latency guarantees." (p. 897, Rocks pick-by-tombstone)
7. "in practice FADE leaves no metadata footprint." (p. 899)
8. "FADE triggers a compaction in a level that has at least one file with expired TTL regardless of its saturation." (p. 899)
9. "FADE ensures that all tombstones inserted into an LSM-tree and flushed to the disk will always be compacted with the last level within the user-defined Dth threshold." (p. 900)
10. "for h = 1. In fact, h = 1 creates the same layout as the state of the art" (p. 901)
11. "We have both block cache and direct I/O enabled and the WAL disabled." (p. 904)
12. "Even when Dth is set to 50% of the workload execution duration, Lethe reduces space amplification by about 48%." (p. 904)
13. "for a workload with even a small fraction (2%) of deletes, it reduces the number of compactions performed by 45%" (p. 904)
14. "Lethe improves lookup performance by up to 17% for workloads with deletes." (p. 904)
15. "For Dth= 50% of the experiment’s run-time, while RocksDB has ∼ 40,000 tombstones (i.e., ∼ 40% of all tombstones inserted) distributed among files that are older than Dth, Lethe persists all deletes within the threshold." (p. 904)
16. "At the end of the experiment, we observe that Lethe writes only 0.7% more data as compared to RocksDB." (p. 905)
17. "For h = 8, the I/O cost for Lethe is 76% lower than that in RocksDB. This comes at the price of a 5× increase in the hashing cost" (p. 905)
18. "For the second workload, which has positive correlation between sort and delete key (≈1), delete tiles have no impact on performance." (p. 905)

## Diálogo teórico

- **R012 / este autor, VLDB’21** — posterior.
  TSD/TSA no menu das 10 = FADE SD/DD medido
  *lá* (+18 / +35% movimento vs LO+1). **Não**
  misturar com o +0.7% amortizado daqui.
- **Monkey [21] / Dostoevsky [23] / R006, R007**
  — FPR e hybrid merge. Lethe não reabre L3;
  Table 2 dá FADE em leveling *e* tiering.
- **X-Engine [39]** — citação de engenheiro:
  delete diário 1/7–1/30 + full-tree. O
  anti-padrão que FADE substitui.
- **Rocks DeleteRange [58] / Callaghan [15]**
  — range nativo + “deletes are fast and slow”.
  Pedra já tem range tombstone; persistência
  continua unbounded.
- **RUM [13]** — delete acrescenta um quarto
  eixo (persistência), não cabe no triângulo.
- **Dong CIDR’17 [27] / R010** — space amp
  ~11% com T=10; Lethe diz que é *um* ponto
  do contínuo.
- **Pebbles [57]** — só citado no fundo.
- **Endure / SILK / ADOC** — não existem
  (2020). Tuner ≠ Dth.

## Relação com Pedra / Montanha

Pedra já tem tombstone de ponto
(`ValueType::Deletion`) e de range
(`delete_range`, merge.rs). Persistência =
quando o compact (Full do par, ou
`compact_for_reads` = *todos* os SST → 1
ficheiro em Lmax com `latest_only`) leva o
tombstone ao fundo. Comentário em `db.rs`
(F20) já avisa: `latest_only` cedo **larga**
tombstones com puts mais velhos ainda em
níveis de baixo — o mesmo invariante do
paper. Sem a_max, sem b, sem TTL, sem Dth.
`MAX_LSM_LEVEL=3`. Sem pick de ficheiro
(L35 ainda MEASURE). Sem delete key
secundária; SST ordenado só na user key.
Bloom por SST, não por página. WAL default
**sync=true** (eval Lethe WAL *off*).

`compact_for_reads` **é** o full-tree que o
X-Engine descreve como pico de I/O. Escape
hatch, não política.

| Lethe (paper) | Pedra | Ledger |
|---------------|-------|--------|
| Persistência unbounded; full-tree = anti-padrão | tombstone + latest_only / compact_for_reads | **L38 `MEASURE`** ganha o paper: FADE (TTL + pick a_max/b) **depois de L35** (precisa file-granular). Gate: produto nomeia Dth (7/30/60 d ou equivalente). Sem SLA, não há o que afinar |
| SO/SD/DD | Full-do-par, 0 pick | DD/TTL-trigger **não** antes do gate. SD sem L35 é no-op |
| “no metadata footprint” | sem num_deletes / seqnum-por-SST no stats | primeiro slice L38: contar tombstones por SST no `DbStats` (eco L41) |
| KiWi tiles, h, BF/página, fence D | sem delete key; layout S só | **L43 `REFUSE`**. h>1 multiplica I/O de point (Table 2 ▼). Correlação S–D≈1 (MVCC seq) → paper diz h=1 |
| Blind-delete BF | delete sempre escreve | **não** filtrar delete com Bloom (falso neg. = tombstone perdido) |
| WAL off no eval | sync=true | números de WA/p99 não transferem |
| T=10, L=3 disco, 1 MB mem | L≤3, 4 MiB, T não-knob | modelo O(T^{L−1}/I) *encolhe* — Dth default de 7 d pode ser folgado *ou* o Full-do-par já empurra. Medir, não copiar 7 d |

- **Não implementar** FADE completo, TTL por
  nível, KiWi, delete tiles, BF por página,
  fence D, Eq. 3, nem portar o patch Rocks.
  **Não** tornar `compact_for_reads` o caminho
  de delete (é o anti-padrão). **Não** reabre
  L3/L37 (tiering). **Não** é L35: pick por
  overlap ≠ pick por tombstone.
- **Teste se o gate L38 abrir:** soak 10%
  deletes + overwrite; idade máxima de
  tombstone vs Dth nomeado; WA
  (bytes_written_sst/ingest) do Full-do-par
  vs SD depois de L35. DST: crash a meio do
  compact TTL = mesmo invariante tmp→rename.
- **Falsificaria L38 (não fazer FADE):** o
  soak com 10% deletes já tem a_max ≪ qualquer
  SLA (L=3 + Full-do-par empurra sozinho) *e*
  space não é o eixo. **Falsificaria L43
  (não recusar KiWi):** um produto com delete
  key ≠ sort key *e* range diário ≥1/30 do
  volume *e* correlação S–D baixa — aí o
  paper é o desenho; ainda assim é *layer*
  (índice / tile), não o kernel sort-key.

Fichas irmãs: R012 (TSD/TSA no menu; +18–35%
é *daquele* bench); R007 (leveling fica);
R010 (espaço>WA; DeleteRange); R013 (file
pick depois de L35); R014 (sem tuner online
de Dth); R017/R016 (stall ≠ delete).

## Avaliação crítica

- **1.17–1.4× / 2.1–9.8× / +4–25% WA** só no
  abstract / contribuições. §5.1: lookup
  **+17%** (=1.17×); space **~48%** em
  Dth=50%; WA fim **+0.7%**, início **1.4×**.
  1.4× *read* e 9.8× *space* não têm figura
  no §5. Não promover o abstract.
- **1 GB, memtable 1 MB, WAL off, SSD 240 GB,
  384 GB RAM.** Micro. Dth como *fração do
  run* (16–50%), não 7 dias de parede.
  Fig. 6F usa Dth=60 s. Amortização “rápida”
  é deste relógio.
- **Deletes 2–10%, uniforme, só keys
  existentes.** Sem o adversário do §3.1
  (hot updates que reciclam tombstone). O
  unbounded do modelo não é o que a Fig. 6
  stressa.
- **KiWi eval** é outro workload (0.001%
  secondary range, 90 GB / 24 h, 1/7). Não
  misturar com FADE 1 GB.
- **Rocks “SoA”** = default trigger+overlap,
  não o pick-by-tombstone que o próprio §3
  cita. Baseline mais fraco que o Rocks de
  produção com esse pick.
- **Não medem:** Pedra-class 1 writer; DST;
  fsync; L=3 Full-do-par (eles *são* L=3
  disco mas partial file-granular); GDPR
  forense (o Dth é de compact, não de disco
  TRIM / vlog blob).
- **Lethe+ TODS’23** e arXiv “updated” podem
  mudar números — não lidos.

## Palavras-chave

lethe; fade; kiwi; tombstone; delete-persistence;
dth; ttl; tsd; tsa; secondary-range-delete;
delete-tile; rocksdb; gdpr

## Fontes

**Primária:** `research/fontes/R031_Sarkar_2020_Lethe.pdf`
(SIGMOD ’20, PDF do autor BU) + `.txt` +
`.pages.txt`.

**Secundárias (não citadas como se fossem
este paper):** `db.rs` (`compact_for_reads`,
`latest_only`, F20); `merge.rs` (range
tombstone); ficha R012 (TSD/TSA +18–35%);
`00_estrategias.md` §1.2.
