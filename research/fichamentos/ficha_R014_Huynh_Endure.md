# Fichamento: R014 — Endure (VLDB’22)

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R014`

## Referência

HUYNH, Andy; CHAUDHARI, Harshal A.; TERZI, Evimaria;
ATHANASSOULIS, Manos. **Endure: A Robust Tuning Paradigm
for LSM Trees Under Workload Uncertainty.** *PVLDB*
15(8):1605–1618, 2022 (VLDB ’22). DOI 10.14778/3529337.3529345.
URL: https://www.vldb.org/pvldb/vol15/p1605-huynh.pdf
Código: https://github.com/BU-DiSC/endure
Estendido: arXiv 2110.13801 [41] (**não** lido; plots extra).
R015 VLDBJ’24 = journal; **não** fichado.

Locus = **p. interna do PDF** (1–14 = PVLDB 1605–1618).
Figs. 1–10 e Tables 1–3 conferidos no texto extraído. Fig. 3–6
barras: números só do parágrafo.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract,
  §§1–10, Figs. 1–10, Tables 1–3, Eqs. 1–16. Referências
  [1]–[85] **não** foram relidas. Monkey [26] / Dostoevsky
  [27] / R012 [67] só pelo que *este* texto afirma (R006,
  R007, R012 já D4). Cliffguard [58] e o arXiv [41] **não**
  lidos. R015 listed.
- **Arquivo lido:** `research/fontes/R014_Huynh_2022_Endure.pdf`
  + `.txt` + `.pages.txt`. Extração com glyphs partidos
  (Endure → E + boxes); quotes reconstruídos contra o
  parágrafo, não contra o OCR partido.
- Como foi lido: PDF local, seção a seção, nesta sessão.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 1–2).** LSM na cloud: o workload
*esperado* (reads vs writes, point vs range) muda.
Tuning estático “point-optimal” (nominal) desconta a
variância e dá desempenho inconsistente. Fig. 1: duas
sessões com o mesmo rácio read/write; a do meio tem
mais range curto → o tuning esperado sofre **2×** I/O.
Mudar o tuning *durante* a execução “is not feasible”
(redistribuir memória + mudar a forma da árvore).
**Endure:** maximiza o pior throughput no
*neighborhood* do workload esperado. Parâmetro de
incerteza (ρ / `d` no texto) controla o raio —
conservador vs optimista. Modelo: até **5×** vs
nominal sob incerteza; perda “negligible” quando
ŵ = w. RocksDB: até **2.4×** throughput.

**§2 Background (p. 2–3).** Leveling vs tiering;
híbridos [27, 28]. Ops: write / point empty (`z0`) /
point nonempty (`z1`) / range (`q`). Bloom por *run*
(na prática por ficheiro). Fence pointers. Tuning
clássico assume w e hardware conhecidos (Monkey
memória [26]; híbridos; compact [5, 52, 66, 67]).

**§3 Problemas (p. 3–4).** Config θ = (T, m_filt,
c) com c ∈ {leveling, tiering}. m_buf = M − m_filt.
Workload w = (z0, z1, q, W)ᵀ, soma 1. Custo
C(w, θ) = wᵀ c(θ) (Eq. 2).

- **Nominal Tuning (P1):** θ# = arg min C(w, θ)
  para um w fixo. É o que Monkey/Dostoevsky fazem
  quando “optimizam para o workload”.
- **Robust Tuning (P2):** θ' = arg min max_{ŵ∈U_w}
  C(ŵ, θ). U_w = {ŵ ≥ 0, ŵᵀ1 = 1, KL(ŵ ‖ w) ≤ ρ}.
  ρ → 0 recupera o nominal.

**§4 Algoritmo (p. 4–5).** Dual via Ben-Tal et al.
[10]; gap zero se c(θ) convexo. Range+tiering
*não* é convexo em T, mas “smooth non-decreasing”
— SLSQP (SciPy) acha o mínimo. <1 s no modelo;
<10 ms no §8. Heurísticas para ρ: max KL entre
históricos e a média; amostrar ranges do DBA; KL
entre “normal” e “off-period”.

**§5 Cost model (p. 5–6).** I/O. FPR Monkey por
nível (Eq. 11). Empty point = soma FPR (×(T−1)
em tiering). Nonempty = P(hit no nível) + FPR
dos anteriores. Range = seeks por run + scan
sequencial (selectivity s_RQ). Write = merges
por nível × assimetria r_w. **Não** modela fence
pointers (admitido no §8: range medido &lt; previsto).

**§6 Benchmark (p. 6–7).** 15 w esperados (Table 2):
uniforme / unimodal / bimodal / trimodal; cada tipo
≥1% para KL finito. Conjunto B: **10 000** w
amostrados (contagens U(0,10k) normalizadas).
ZippyDB FAST’20 [17]: 78% get / 19% write / 3%
range ≈ w11.

**§7 Modelo (p. 7–8).** 10 M entries × 1 KB, 10 GB
RAM (qualitativo estável a outros N, M). 15 × 15
valores de ρ ∈ (0, 4] passo 0.25 × 10k = &gt;2 M
comparações.

- Unimodal/bimodal/trimodal: robusto ρ≥0.5 dá
  **&gt;95%** melhoria média de delta-throughput
  vs nominal em B.
- Uniforme w0: nominal **ganha ~5%** (ρ alto =
  pessimista demais; Fig. 2: o mesmo ρ cobre mais
  área no simplex).
- ρ=0 ≈ nominal.
- ρ↑: T desce (antecipa writes) se w era read-heavy
  (Fig. 4, w11 = 1% write). Throughput *range*
  (pior−melhor em B) **desce** — mais consistente
  (Fig. 5).
- Rule of thumb: ρ = max KL entre pares de w
  observados.

**§8 RocksDB (p. 9–12).** 2× Xeon Gold 6230, 384 GB,
1 TB Dell P4510 NVMe, CentOS 7.9, page 4 KB.
Leveling e tiering *clássicos* via event hooks
(sem micro-opts Rocks). Monkey bits/nível. Direct
I/O, **block cache off**. 10 M keys 1 KB (16-bit
key aleatória). 200 k queries/sessão, 6 M no total.
Bulk load 30 min; workload ~5 min. Range curto
(0–2 páginas/nível). Writes = keys *novas*.

- Tuning <10 ms.
- Fig. 7–9: modelo ≈ sistema em I/O relativo;
  range discrepante por fences.
- Writes: nominal com T grande = níveis enormes
  + stalls; robusto T menor = estável. Até
  **90%** menos I/O e latência.
- Table 3 (ρ “óptimo”): robusto ganha em **10/15**
  w; 2 empatam (0.0); 2 perdem **−0.1** (w13,
  w14). **Todos** os θ' são **Leveling**. Vários
  nominais são Tiering (w4, w7, w9, w12) e o
  robusto *vira* L. w4: (17, 3.2, T) → (4.6, 1.0,
  L), δ=1.5. w11: (47, 4.7, L) → (11, 1.9, L),
  δ=1.4.
- Fig. 8: ŵ≈w (KL&lt;0.2) → robusto **+20%**
  latência (o preço quando *não* há incerteza).
- Fig. 10: N de 10²… cresce; gap robusto/nominal
  **estável**; níveis iniciais iguais (buf cresce
  com N).
- §8.4: 700 tunings × B ≈ **8.6 M** comparações
  modelo; robusto ganha **&gt;80%**. 300+ no
  Rocks confirmam.
- **“Leveling is more Robust than Tiering.”**
  Alinha com a indústria (leveling / hybrid, não
  pure tiering). “Robust tuning should always be
  employed … unless the future workload
  distribution is known with absolute certainty.”

**§9–10 (p. 12).** Related: auto-index, self-driving
[4, 56, 62], self-designing [42, 44–46] — todos
assumem w honesto. Cliffguard [58] = robust
physical design sem closed form; no objectivo LSM
**não converge**. Conclusão: DBA usa Endure em
segundos, sem soak caro.

## Tese central e argumento

O sintonizador *point-optimal* (nominal) sobre-ajusta
a um w. Sob deriva (mesmo rácio R/W, mais range)
o I/O dispara (Fig. 1). Endure escolhe (T, bits,
leveling|tiering) que maximiza o pior caso num
bola KL. ρ=0 = Monkey/Dostoevsky clássicos.

O paper **não** pede um tuner online. Diz o
contrário: retune contínuo é inviável (p. 2).
Também **não** pede tiering: o robusto *sempre*
escolhe leveling (Table 3).

## Estrutura do texto

| § | p. | Conteúdo |
|---|---:|----------|
| Abstract / 1 | 1 | Fig. 1 2×; 5× / 2.4× |
| 2 LSM | 2 | leveling/tiering; Bloom |
| 3 P1/P2 | 3 | nominal vs robust; KL |
| 4 dual | 4 | Ben-Tal; SLSQP; ρ |
| 5 model | 5 | Monkey FPR; 4 custos |
| 6 bench | 6 | Table 2; 10k B |
| 7 modelo | 7 | 95%; ρ; Fig. 3–6 |
| 8 Rocks | 9 | 90%; Table 3; L≻T |
| 9–10 | 12 | Cliffguard; conclusão |

## Conceitos-chave

- **Nominal tuning.** arg min C(w, θ) para um w.
  Termo do paper (P1).
- **Robust tuning.** arg min max_{ŵ∈U} C(ŵ, θ).
- **U_w / ρ.** Bola KL à volta de w (Eq. 5).
- **θ = (T, m_filt, c).** Únicos knobs. Policy
  só L ou T — **não** Fluid/LL.
- **Normalized delta throughput.** (1/C₂ − 1/C₁)/
  (1/C₁).
- **Throughput range.** max−min 1/C em B
  (consistência).
- **Monkey allocation.** Eq. 11; input do modelo,
  não um resultado novo.

## Citações relevantes

1. "the robust tunings output by Endure lead up to a 5× improvement in throughput in the presence of uncertainty." (p. 1)
2. "Endure tunings have negligible performance loss when the observed workload exactly matches the expected one." (p. 1)
3. "it is not feasible to continually change tunings during execution, as it requires redistribution of the allocated memory between different components of the tree and potentially changing its shape." (p. 2)
4. "As the KL-divergence boundary condition approaches zero, our problem becomes equivalent to the classical optimization problem (henceforth referred to as the Nominal Tuning problem)." (p. 2)
5. "Solving this problem outputs … a robust tuning configuration for a given input in less than a second." (p. 5)
6. "This type of workload breakdown … ZippyDB … 78% gets, 19% writes, and 3% range reads." (p. 7, [17])
7. "the normalized delta throughput shows over 95% improvement on average … for robust tunings with d ≥ 0.5" (p. 8; unimodal/bimodal/trimodal)
8. "For uniform expected workload, we observe that the nominal tuning outperforms the robust tuning by a modest 5%." (p. 8)
9. "the robust tunings for d = 0 i.e., zero uncertainty, are very close to the nominal tunings." (p. 8)
10. "Solving for nominal and robust tuning takes less than 10ms" (p. 9)
11. "the robust tuning reduces I/O and latency by up to 90%." (p. 10)
12. "all robust tunings suggest leveling as the compaction policy." (p. 12, Table 3)
13. "Leveling is “more” Robust than Tiering." (p. 12)
14. "the robust tuning should always be employed when tuning an LSM tree, unless the future workload distribution is known with absolute certainty." (p. 12)
15. "Robust tunings comprehensively outperform the nominal tunings in over 80% of these comparisons." (p. 12; 8.6 M)
16. "In Figure 8 … resulting in a latency increase of 20%." (p. 11; ŵ≈w)
17. "Their method [Cliffguard] … fails to converge even after extensive hyperparameter search." (p. 12)
18. Table 3 w4: nominal (17, 3.2, T) → robust (4.6, 1.0, L), δ=1.5 (p. 11)
19. Table 3 w11: (47, 4.7, L) → (11, 1.9, L), δ=1.4 (p. 11)
20. "up to 2.4× throughput speedups" (p. 2)

## Diálogo teórico

- **Monkey [26] / R006** — FPR por nível *dentro*
  do cost model (Eq. 11). Endure *usa* L2; não o
  prova de novo.
- **Dostoevsky [27] / LSM-Bush [28] / R007** —
  híbridos citados como “fine-grained”. Endure
  **só** L vs T; não navega (K,Z).
- **R012 [67]** — design space de compact; Endure
  é o eixo *T + memória + policy*, não granularity.
- **R010 [17] / Facebook FAST’20** — ZippyDB 78/19/3.
- **Cliffguard [58] + Bertsimas [12]** — robust
  physical design; não fecha no objectivo LSM.
- **Self-driving [4, 56, 62]** — ML; “yield
  suboptimal results when … information is
  inaccurate” (p. 12).
- **Ben-Tal [10]** — dual robusto.

## Relação com Pedra / Montanha

Pedra hoje: `MAX_LSM_LEVEL=3`, Bloom **uniforme**
10 bits (L2 MEASURE), compact Full-do-par, **sem**
T primeiro-classe, **sem** switch L/T (L3 REFUSE
LL; L37 REFUSE universal). Não há θ = (T, m_filt,
c) para Endure sintonizar.

| Endure (paper) | Pedra | Ledger |
|----------------|-------|--------|
| Nominal = point-optimal para um w | não temos tuner | **não** criar um |
| Robusto = pior caso na bola KL | sem knobs | **L12 `MEASURE` src=ficha**: default *estático* robusto **se** existirem ≥2 knobs {T, Monkey bits, policy}. Hoje = 0. **Não** SHIP tuner |
| Retune online inviável (p. 2) | DST-hostil mudar forma | confirma L36 (não auto-switch) |
| Robusto **sempre** Leveling (Table 3) | L37 / L3 | **confirma REFUSE** tiering/LL como default. Endure *piora* o caso para T |
| Monkey FPR no modelo | L2 MEASURE | **não reabre** L2. Hardware/L do R006 intactos |
| 5× / 2.4× / 90% | Rocks + 10 M × 1 KB + cache off + NVMe + 384 GB | não é `benches/baseline` |
| ρ=0 ≡ nominal | — | sem incerteza nomeada, Endure = Monkey clássico |

- **Não implementar** Endure, SLSQP no binário,
  switch L/T, alocador Monkey “porque o robusto
  precisa”.
- **Teste se L12:** primeiro um knob real (T ou
  bits por nível). Depois: dois w (DCS-like vs
  scan-like), ρ = KL(w1‖w0), θ' vs θ# no mesmo
  soak; DST não entra (tuning é offline).
- **Falsificaria L12 (não fazer tuner):** o estado
  actual — um só compact, 10 bits, L=3 — já é o
  “leveling raso” que Table 3 escolhe. **Falsificaria
  “nunca robustecer”:** L2 ou T shipped *e* um
  named pair (w, ŵ) em que θ# perde &gt;2× para θ'
  no `baseline`.

Fichas irmãs: R006 (Monkey; L2); R007 (LL REFUSE);
R012 (4 primitivas; L36); R013 (granularity, outro
eixo). R015 = journal, não lido.

## Avaliação crítica

- **Cost model I/O, cache off, fences omitidos.**
  Range do sistema &lt; modelo (p. 10). Write cost
  assume “quase sem overlap” — Zipf-hot real é
  outro mundo (R018).
- **Policy ∈ {L, T} apenas.** Fluid/Spooky/LO+1
  não existem no θ. “Robustez” é *neste* simplex.
- **10 M × 1 KB, 16-bit keys.** Árvore rasa; Fig. 10
  admite que o nº de níveis é o que manda — e o
  buf cresce com N para o *manter*.
- **384 GB RAM / 1 TB NVMe / 2 sockets.** Não é
  embed Pedra.
- **ŵ amostrado U(0,10k)⁴.** Não é deriva *temporal*
  de uma app (Fig. 1 é o único vestígio).
- **Table 3 ρ “óptimo”** — escolhido *depois* de
  ver B. Não é um DBA cego.
- **+20% latência quando ŵ=w** (Fig. 8) — o paper
  diz “negligible” no abstract; o sistema mostra
  20%.
- **Não medem:** Pedra L=3; compact Full-do-par;
  bloom uniforme; crash a meio de *aplicar* um
  θ novo (eles nem aplicam online).

## Palavras-chave

endure; robust-tuning; workload-uncertainty;
kl-divergence; nominal; leveling; tiering;
monkey; size-ratio; rocksdb

## Fontes

**Primária:** `research/fontes/R014_Huynh_2022_Endure.pdf`
(PVLDB 15(8), vldb.org) + `.txt` + `.pages.txt`.

**Secundárias (não citadas como se fossem este
paper):** fichas R006, R007, R012; RFC-0012.
R015 / arXiv [41] / Cliffguard **não** lidos.
