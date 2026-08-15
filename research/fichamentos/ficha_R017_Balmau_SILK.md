# Fichamento: R017 — SILK (ATC’19)

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R017`

## Referência

BALMAU, Oana; DINU, Florin; ZWAENEPOEL, Willy;
GUPTA, Karan; CHANDHIRAMOORTHI, Ravishankar;
DIDONA, Diego. **SILK: Preventing Latency Spikes in
Log-Structured Merge Key-Value Stores.** In: *2019
USENIX Annual Technical Conference (ATC ’19)*, 10–12
July 2019, Renton, WA. pp. 753–766 (+ refs 767).
URL: https://www.usenix.org/system/files/atc19-balmau.pdf
Código: https://github.com/theoanab/SILK-USENIXATC2019

Locus = **p. USENIX** (753–767). Figs. 1–13 e Table 1
conferidos no texto extraído. Fig. 8/12 barras: números
só do parágrafo. SILK+ (TOCS’20) **não** lido.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract,
  §§1–8, Figs. 1–13, Table 1. Referências [1]–[44] **não**
  foram relidas. TRIAD [4], PebblesDB [39], Rocks rate
  limiter [19, 22], bLSM [41], WiscKey [32], HashKV [10],
  Monkey [12], Dostoevsky [13] só pelo que *este* texto
  afirma (R006, R007, R018 já D4). ADOC (R016) **não**
  existe ainda (2023); ficha R016 agora D4.
- **Arquivo lido:** `research/fontes/R017_Balmau_2019_SILK.pdf`
  + `.txt` + `.pages.txt`.
- Como foi lido: PDF local, seção a seção, nesta sessão.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 753–754).** Compaction *já* foi
atacada para throughput (rate-limit, pick, fragmented
LSM). Este paper é **latência**, não ops/s. Causa
das caudas: interferência entre writes do cliente,
flushes e compactions. Scheduler de I/O com três
técnicas: (1) largura de banda extra para trabalho
interno em carga baixa; (2) priorizar flush e
compact L0→L1; (3) preemptar compactos altos.
SILK = RocksDB + este scheduler (conceitos
portáveis). Até **duas ordens de magnitude** no
p99 vs Rocks e TRIAD, sem estragar o resto.
Nutanix: LSM para metadata da plataforma.

**§2 LSM (p. 754–755).** Cm (dezenas de MB) →
flush para L0 (ranges sobrepostos); L1+ disjoint;
Clog opcional. Cliente FIFO num pool; interno
FIFO noutro. L0→L1 **não** paraleliza. Rocks:
rate limiter fixo ou auto MIMD [19] — este
aumenta BW quando há *mais trabalho interno*,
cego à carga do cliente. TRIAD: hot em Cm +
compact L0→L1 por overlap. Pebbles: FLSM /
guards; compact só no último nível.

**§3 Requisitos (p. 755).** (1) p99 baixo (fan-out).
(2) throughput previsível. (3) RAM pequena
(co-locate). Problema: burst de writes + compact
de dezenas de GB durante dezenas de minutos.
Flush lento → Cm cheia → write bloqueia. L0
cheia → flush pára → halt. Reads enfileiram
atrás dos writes no pool.

**§4 Estudo (p. 755–758).** YCSB A 50:50, 1 KB,
uniforme, 18 Kops/s. Rocks: 2×128 MB memtable;
outros ≤1 GB RAM; até 10 internos em paralelo.
p99 por janela de 1 s.

- **Fig. 1.** Rocks vs Rocks *sem* flush/compact
  (descarta Cm). p99 Rocks **2–4 ordens** acima.
  Spikes **não** no p50/p90. Read e write
  disparam juntos (write bloqueia; read atrás).
- **Fig. 2.** Spike porque L0 = 10 SST: muitos
  L1→L2 em paralelo roubam I/O; um só L0→L1
  atrasado; flush pára.
- **Fig. 3.** L0 com 7 SST; flush de 5 s (típico
  1–2 s) porque 7 compactos L1→L2; Cm enche.
- **Fig. 4.** Rate-limit interno 50/75/90 MB/s:
  mais BW = spike *mais tarde* (900 s a 50 MB/s);
  depois piora — compactos acumulam e disparam
  juntos.
- **§4.4.** Cm até 1 GB (2×500 ou 10×100);
  1 vs 10 flush threads. Melhor: 10 memtables +
  1 flush. Spikes *mais tarde*, sempre voltam.
- **Fig. 5 TRIAD.** Sem spike ~1000 s; depois
  frequentes (adiar L0→L1 adia L alto → rajada).
- **Fig. 6 Pebbles.** p99 bom até OOM (~10 500 s)
  — sem compact. 95:5: às 8 h o compact do
  último nível **para o sistema** (todas as
  threads ocupadas; guards não descem).
- **Lições:** (1) p99 = write bloqueado por Cm
  cheia (L0 full *ou* flush lento). Internos
  perto de Cm são críticos. (2) Limitar BW
  adia e *agrava*. (3) Menos compact (TRIAD /
  Pebbles) adia e agrava. Testes curtos mentem.

**§5 SILK (p. 758–759).** Três princípios.

1. **BW oportunista.** Monitor 10 ms; I = T − C − ε
   (T=200 MB/s no eval). Ajuste só se Δ ≥ 10 MB/s.
   Mínimo configurável para flush e L0→L1
   (50 MB/s no eval).
2. **Prioridade.** Flush > L0→L1 > L≥1. Flush
   tem pool próprio. L0→L1 nunca pausa: se
   preciso, sobe ao pool high e partilha o
   mínimo com flush. L0→L0 (Rocks recente)
   tratado como L0→L1.
3. **Preempt.** Compacto alto cede a L0→L1
   (pick aleatório). Trabalho parcial pode ser
   descartado; “não impactou”. 4 threads
   internos (não = nº de cores): num disco
   200 MB/s, 4 × fatia ainda acaba depressa.

Dois memtables + 1 flush thread.

**§6 Eval (p. 759–763).** RocksDB-SILK e
TRIAD-SILK. 20-core Xeon (2×10 @ 2.8 GHz),
256 GB RAM, **960 GB Samsung 843T**, cgroup
**1 GB**. db_bench. Auto-tuned Rocks [19] na
comparação. 128 MB × 2 memtables. T=200 MB/s;
mínimo flush/L0 50 MB/s. L0-slowdown/stop
**muito altos** (não interferir na medida).
4 threads internas; 16 cores pinados (8 worker
+ 8 interno). Compressão e **commit log OFF**
(dizem que não muda as *diferenças*).

**6.2 Nutanix (Fig. 8A, 9–11).** Trace 24 h,
write-dominated, item mediano 400 B, replay à
taxa original. SILK: **2 ordens** melhor p99
que Rocks auto-tuned; **3 ordens** que Rocks e
TRIAD. Throughput cola à carga; Rocks oscila
(filas → rajadas). Fig. 9: SILK **nunca**
stalla writes e **sempre** flusha Cm a tempo.
TRIAD: flush atrasado **69×**. Auto-tuned:
stalla **1%** do tempo. 24 h (Fig. 10): estável;
~**3 TB** compactados; ≤3 compactos à espera;
0 stall. Fig. 11: 200 s — interno sobe quando
o cliente desce.

**6.3 YCSB (Table 1, Figs. 12–13).** TRIAD-SILK.
8 B key + 1024 B value. Uniforme: throughput
SILK −**≤7%** (pior em F). Zipf: −**≤4%**
(quase tudo em RAM). Read-heavy −≤5%. Mediana
≈ TRIAD; E-zipf **+5%**. p99 melhor em *todos*;
mínimo −5% (E); write-heavy até **2 ordens**.
L0 Bloom + first-hit: adiar L0→L1 quase não
castiga Get.

**6.4 Picos (Fig. 8B).** Vale 10 Kops, pico
40 Kops (mais que produção). Picos 10/50/100 s
+ vale 10 s, 50:50: p99 baixo. Pico longo:
degrada ~**500 s** (300 s de pico) a 90% write;
~**700 s** (500 s de pico) a 50% write.
Produção: picos ≤400 s, 50% write, metade da
carga — dentro da janela.

**6.5 Breakdown (Fig. 8C).** Só rate-limit:
interfere pouco no início; depois L0→L1
atrasa. Só prioridade/preempt: árvore rasa OK;
sem cap de BW a interferência volta. Os dois
juntos é que aguentam.

**§7–8 (p. 763–764).** Throughput papers
(WiscKey, HashKV, TRIAD, Monkey, Dostoevsky,
Pebbles…) *reduzem trabalho*, não a
interferência. SILK é complementar (aplicam
em Rocks *e* TRIAD). Offload de compact para
outro servidor [1] tira interferência mas
distribui o KV. bLSM throttle pode stallar
o write do user; SILK faz o interno no vale.
Conclusão: scheduler, não árvore nova.

## Tese central e argumento

p99 não se compra com menos compact. Compra-se
com **coordenação** entre carga do cliente e
trabalho interno: flush e L0→L1 primeiro;
compacto alto no vale; preempt se o L0 aperta.
Rate-limit cego e “adiar compact” *adiam o
spike*. Teste curto mente (lição 3).

O paper **não** pede um LSM novo nem 16
threads de compact. Pede um scheduler que o
DST possa desligar.

## Estrutura do texto

| § | p. | Conteúdo |
|---|---:|----------|
| Abstract / 1 | 753 | p99; 3 técnicas; 100× |
| 2 LSM | 754 | Rocks / TRIAD / Pebbles |
| 3 reqs | 755 | fan-out; RAM |
| 4 estudo | 755 | Figs. 1–6; 3 lições |
| 5 SILK | 758 | BW; prioridade; preempt |
| 6.1–6.2 | 759 | setup; Nutanix 24 h |
| 6.3–6.5 | 762 | YCSB; picos; breakdown |
| 7–8 | 763 | related; conclusão |

## Conceitos-chave

- **I/O scheduler LSM.** Coordena cliente vs
  flush vs compact (p. 754). Termo do paper.
- **Interferência.** Não é “WA alto”; é o
  *quando* o I/O interno corre.
- **Flush > L0→L1 > L≥1.** Ordem de criticidade
  (lição 1).
- **BW oportunista.** I = T − C − ε no vale
  (p. 758).
- **Preempt.** Compacto alto cede a L0→L1.
- **Auto-tuned rate limiter [19].** Sobe BW com
  *backlog interno* — cego ao cliente.
- **L0-slowdown / L0-stop.** No eval, deitados
  para não mascarar o p99.

## Citações relevantes

1. "The root cause of these high tail latencies is interference between client writes, flushes and compactions." (p. 753)
2. "SILK achieves up to two orders of magnitude lower 99th percentile latencies than RocksDB and TRIAD" (p. 753)
3. "The techniques that have been proposed for optimizing throughput do not address this issue, and in fact in some cases exacerbate it." (p. 753)
4. "The 99th percentile latency of operations in RocksDB is 2 to 4 orders of magnitude higher than in the system without internal operations." (p. 756, Fig. 1)
5. "These spikes are not present at the 50th and 90th percentiles of the latency distribution." (p. 756)
6. "The main culprit for the latency spikes is the fact that writes get blocked by virtue of Cm filling up." (p. 756)
7. "Limiting the rate of compactions is also insufficient, because they can lead to the lowest level of the tree filling up, stalling flushes, and in turn stalling writes." (p. 754)
8. "Lesson 2) Simply limiting bandwidth for internal operations does not solve the problem … and can in fact exacerbate it in the long run." (p. 758)
9. "it is essential to run performance tests for an extended amount of time, lest these issues go undetected." (p. 758)
10. "SILK never stalls writes and can always flush Cm as soon as it fills up." (p. 762, Fig. 9)
11. "TRIAD has the most problems flushing Cm on time – the flush is delayed 69 times" (p. 762)
12. "The auto-tuned version of RocksDB … spends … 1% of the total experiment time [stalling writes]." (p. 762)
13. "SILK obtains two orders of magnitude lower tail latency than the auto-tuned RocksDB, and three orders of magnitude better than RocksDB and TRIAD" (p. 762)
14. "SILK has low impact on throughput … at most 7%." (p. 762, YCSB uniforme)
15. "SILK’s benefits in terms of tail latency are most pronounced in write-dominated workloads, where the latency is decreased by up to two orders of magnitude." (p. 763)
16. "SILK’s performance starts to degrade at around 500 seconds (300 seconds of peak) for the 90% writes workload" (p. 763)
17. "On their own, neither of the two techniques is able to sustain the client load." (p. 763, Fig. 8C)
18. "the number of compaction threads should instead depend on the total drive I/O bandwidth" (p. 759)
19. "level0-slowdown and level0-stop parameters … are configured to very large values in all data stores so as not to artificially interfere with the measured latency." (p. 761)
20. "These techniques decrease the amount of work performed during internal operations … they do not avoid the interference with user operations while internal operations execute." (p. 764)

## Diálogo teórico

- **TRIAD [4]** — menos compact; p99 *pior* no
  longo prazo (Fig. 5). SILK aplica-se *em cima*.
- **PebblesDB [39] / R008 listed** — p99 bom até
  o compact do L último parar tudo (§4.6).
- **Rocks rate limiter [19, 22]** — auto MIMD
  olha o backlog interno, não o cliente.
- **bLSM [41]** — throttle de níveis; pode
  stallar o write do user (p. 764).
- **WiscKey [32] / HashKV [10] / R005, R018** —
  menos bytes no compact; interferência fica.
- **Monkey [12] / Dostoevsky [13] / R006, R007**
  — knobs; outro eixo (Endure R014).
- **Ahmad VLDB’15 [1]** — compact noutro
  servidor = distribui o KV (L16 cousin).
- **ADOC R016** — posterior; agora ficha D4. Tuner
  de caudal (threads+batch), não scheduler. L42
  REFUSE; taxonomia em L41.

## Relação com Pedra / Montanha

Pedra `Db`: flush no caminho `&mut` (bloqueia o
writer). `ConcurrentDb`: write-group + dual
memtable + pipeline de flush (I/O sem o write
lock); compact ainda exclusivo no *install* do
SST. Sem rate limiter, sem nomes de stall, sem
prioridade flush vs L0 vs L≥1, sem preempt.
`L0_COMPACTION_TRIGGER=4` dispara compact — **não**
é L0-stop. DCS/leases sentem p99, não ops/s.

| SILK (paper) | Pedra | Ledger |
|--------------|-------|--------|
| p99 = interferência, não WA | compact Full-do-par já é um job grande | **L41 `MEASURE`**: nomes de stall + flush > compact L0 > L≥1. Gate: soak em que p99 write ≥10× p50 *e* o motivo é Cm/L0, não fsync |
| Rate-limit cego agrava | não temos limiter | **não** copiar o auto-tuner Rocks [19] |
| BW no vale (T−C−ε) | DST precisa poder *desligar* I/O fake | limiter só se for `Env`-visível e off no sim |
| Preempt compacto alto | um compact de cada vez no `&mut` | preempt é no-op até haver bg compact + L0 |
| 4 threads ≠ ncores | 00_estrategias já pedia isto | confirma; não 16 bg (R013) |
| Teste curto mente | `benches/baseline` é curto | soak nomeado (minutos), não 10 s |
| TRIAD/Pebbles “menos compact” | L37/L3 | **não** reabre: adiar compact ≠ matar p99 |

- **Não implementar** o scheduler SILK completo,
  preempt de compact, monitor 10 ms, 200 MB/s
  cap. **Não** ligar L0-stop sem métrica.
- **Primeiro slice se L41:** `DbStats` com
  contadores `stall_memtable_full` /
  `stall_l0` / `flush_wait_compact` (zero
  hoje = ou não medimos ou não stallamos).
- **Teste:** soak 50:50 com picos; p99 por
  janela de 1 s como o paper; `FailingEnv`
  *não* deve ver o limiter (off).
- **Falsificaria L41 (não fazer):** o soak
  Pedra com dual-mem + pipeline já tem p99 ≈
  p50 (writer não espera flush). **Falsificaria
  “nunca scheduler”:** o mesmo soak com p99
  ≥10× p50 *e* os novos contadores apontam
  Cm/L0, não WAL sync.

Fichas irmãs: R012 (compact > write no *bench*
deles — o contrário de SILK); R013 (16 bg
misturam níveis — exactamente Fig. 2–3);
R014 (tuner ≠ scheduler); R018 (menos bytes,
mesma interferência). R016 ADOC (D4): complementar,
não substituto; tuner REFUSE (L42).

## Avaliação crítica

- **Samsung 843T 960 GB, 1 GB cgroup, WAL e
  compressão OFF.** p99 é *deste* disco e
  desta RAM. Pedra default *sync=true* — o
  fsync do WAL pode dominar o p99 e o paper
  não o mede.
- **L0-stop deitado.** Produção Rocks usa
  slowdown/stop; o p99 “sem SILK” pode ser
  *pior* ou *diferente* com esses knobs.
- **“3 ordens vs Rocks”** no trace Nutanix;
  YCSB write-heavy “até 2 ordens”. Abstract
  generaliza “up to two”.
- **Picos sintéticos 40 Kops** > produção;
  a janela 300–500 s é o teto, não um SLA.
- **Preempt descarta trabalho** — sem número
  de bytes deitados fora.
- **Não medem:** Pedra-class 1 writer; DST;
  multi-tenant no mesmo `Env`; NVMe moderno
  com mais BW (o argumento das 4 threads
  muda).
- **SILK+ TOCS’20** (heterogéneo) não está
  aqui.

## Palavras-chave

silk; tail-latency; stall; flush; compaction;
io-scheduler; l0; rocksdb; triad; pebblesdb;
rate-limiter; preemption

## Fontes

**Primária:** `research/fontes/R017_Balmau_2019_SILK.pdf`
(USENIX ATC ’19) + `.txt` + `.pages.txt`.

**Secundárias (não citadas como se fossem este
paper):** `concurrent.rs` (write-group / flush
pipeline); `00_estrategias.md` §1.5; fichas
R012–R014. SILK+ **não** lido. ADOC = ficha R016.
