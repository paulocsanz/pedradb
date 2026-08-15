# Fichamento: R005 — WiscKey (key-value separation)

**Status:** D4
**Lido em:** 2026-08-14
**Catalog:** [CATALOG.md](../CATALOG.md) · `R005`

## Referência

LU, Lanyue; PILLAI, Thanumalayan Sankaranarayana; ARPACI-DUSSEAU, Andrea C.;
ARPACI-DUSSEAU, Remzi H. **WiscKey: Separating Keys from Values in
SSD-conscious Storage.** In: *14th USENIX Conference on File and Storage
Technologies (FAST ’16)*, Santa Clara, CA, 22–25 Feb. 2016. p. 133–148.
ISBN 978-1-931971-28-7.
URL: https://www.usenix.org/system/files/conference/fast16/fast16-papers-lu.pdf

Páginas internas do PDF: 1–16. Locus abaixo usa **§ + p. interna** (rodapé
do paper) e, quando útil, a página FAST.

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra (corpo §§1–6 + figuras 1–15 +
  Table 1). Referências [1]–[54] **não** foram relidas como obras; só o que
  o texto afirma sobre elas.
- **Arquivo lido:** `docs/references/wisckey-fast2016.pdf` + extração
  `docs/references/wisckey-fast2016.txt` (mesmo conteúdo; citações
  conferidas no `.txt` contra o fluxo do PDF).
- Como foi lido: PDF local + txt de extração, seção a seção, nesta sessão.
  Figuras 1, 4, 5 descritas no texto; números de Fig. 2–3 e 7–15 tirados das
  legendas e do parágrafo que as comenta.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**§1 Introduction (p. 1–2 / FAST 133–134).** LSM-trees viraram o estado da
arte para KV write-intensive (BigTable, LevelDB, Cassandra, HBase, RocksDB,
PNUTS, Riak). O ganho sobre B-tree é acesso sequencial nas writes. O preço:
o mesmo par é lido e escrito muitas vezes; “this I/O amplification in
typical LSM-trees can reach a factor of 50x or higher” (p. 1). Esse
trade-off faz sentido em HDD (random ≫ 100× sequential). Em SSD o paper
destaca três diferenças: (i) gap random/sequential menor; (ii) paralelismo
interno; (iii) wear por write amp. Com LSM clássico em SSD, “reducing
throughput by 90% and increasing write load by a factor over 10” (p. 1).

WiscKey deriva de LevelDB. Ideia central: só as keys ficam sorted na LSM;
values vão a um log separado. Compact deixa de mover values. A LSM encolhe
→ menos reads e melhor cache. Três problemas que o desenho tem de resolver:
scan vira random I/O (mitigado com paralelismo SSD); GC do value-log;
crash consistency (apêndice nunca produz lixo no meio do ficheiro).

O próprio paper **não** afirma vitória universal: “if small values are
written in random order, and a large dataset is range-queried sequentially,
WiscKey performs worse than LevelDB” (p. 2). Diz que isso não reflete
YCSB (ranges curtos) e que log reorganization ajudaria.

**§2 Background (p. 2–4 / FAST 134–136).** Recapitula LSM (C0 memória,
C1…Ck disco, compact entre níveis adjacentes) e LevelDB 1.18: WAL +
memtable + immutable + L0–L6, size ratio 10, L0 pode overlap, slowdown se
L0 > 8 ficheiros (p. 3). Write amp: mover um ficheiro Li−1→Li pode ler até
10 ficheiros; L0→L6 “write amplification can be over 50” (p. 3). Read amp
pior caso: 8 ficheiros em L0 + 6 níveis = 14; dentro de cada SST,
index + bloom + data (ex. 16+4+4 KB para um KV de 1 KB) → 24×14 = 336
(p. 3). Medição própria (Fig. 2, key 16 B / value 1 KB): load 1 GB WA 3.1
/ RA 8.2; load 100 GB WA 14 / RA 327 (p. 3–4).

Fig. 3 (Samsung 840 EVO, ficheiro 100 GB em ext4): random 1-thread chega a
metade do sequential em 256 KB; 32 threads igualam sequential acima de
16 KB (p. 4). Conclusão do §2.4: LSM em SSD “waste a large percentage of
device bandwidth” (p. 4).

**§3 Design (p. 4–8 / FAST 136–140).** Quatro ideias: (1) separar K/V;
(2) prefetch paralelo no scan; (3) GC + crash do vLog; (4) **remover o
WAL da LSM** porque o vLog já guarda keys.

§3.1 goals: low WA, low RA, SSD-optimized, API rica (range + snapshots),
tamanhos “realistas” (keys ~16 B; values 100 B a >4 KB) (p. 4–5).

§3.2: compact “only needs to sort keys” (p. 5). Pointer
`<vLog-offset, value-size>` na LSM. Cálculo-exemplo: key 16 B, value 1 KB,
WA keys=10, WA values=1 → WA efetivo (10×16+1024)/(16+1024) = **1.14**
(p. 5). LSM de um dataset de 100 GB ≈ 2 GB (12 B por localização) —
cacheável em servidores com >100 GB RAM (p. 5). Put: append value no vLog,
depois insert key+addr na LSM. Delete: só a LSM; values inválidos ficam
para GC. Get: LSM depois random read no vLog.

§3.3.1 Range: iterator LevelDB; WiscKey detecta sequência e mete addrs
numa queue; **32 threads** no pool (p. 6 e §3.5).

§3.3.2 GC: compact da LSM só limpa keys. GC *offline* (scan da LSM por
addrs válidos) é “too heavyweight” (p. 6). GC *online*: o vLog passa a
guardar o tuple `(key size, value size, key, value)` (Fig. 5, p. 6).
Head = append; tail = início do GC. Lê um chunk (vários MB) da tail,
valida cada value na LSM, re-append válidos na head, fsync vLog, grava
novos addrs + `'tail'` na LSM de forma síncrona, *depois* liberta o
espaço (hole-punch `fallocate`) (p. 6–7). Configurável periódica /
threshold / offline.

§3.3.3 Crash: propriedade de ext4/btrfs/xfs — append após crash é
**prefixo** dos bytes novos, nunca lixo no meio (citam Pillai/ALICE [45])
(p. 7). Se a key não está na LSM, o value órfão no vLog é GC depois. Se
a key está, verifica range do vLog e se o value corresponde à key; senão
apaga a key e devolve not-found. Sync Put: fsync vLog **antes** do insert
síncrono na LSM (p. 7).

§3.4.1 Buffer userspace no vLog (Fig. 6: write() <4 KB é caro em ext4)
(p. 7). Mesma garantia que LevelDB para inserts async (buffer pode
perder-se).

§3.4.2 **Drop do WAL da LSM:** keys já estão no vLog por causa do GC.
Recuperação: scan do vLog a partir do `'head'` periodicamente gravado na
LSM, não do ficheiro inteiro (p. 8).

§3.5 Implementation: fork de **LevelDB 1.18**; `posix_fadvise` no vLog;
32 threads de prefetch; hole-punch; ficheiro único (64 TB ext4) ou log
circular “if necessary” (p. 8).

**§4 Evaluation (p. 8–12 / FAST 140–144).** Hardware fixo: 2× Xeon
E5-2667 v2 3.30 GHz, **64 GB RAM**, Linux 3.14, ext4, **Samsung 840 EVO
500 GB** (seq R 500 / W 400 MB/s) (p. 8). Key sempre 16 B; compressão
**desligada**. Baseline micro: LevelDB (`db_bench`). Macro: LevelDB +
RocksDB default + WiscKey ± GC sempre ligado.

- Sequential load 100 GB (Fig. 7): WiscKey satura o device para values
  ≥4 KB; 3× mais rápido que LevelDB mesmo em values pequenos (sem WAL
  LSM + buffer) (p. 8–9).
- Random load 100 GB (Fig. 9–10): LevelDB 2–4.1 MB/s; WiscKey **46×**
  (1 KB) e **111×** (4 KB) vs LevelDB; WA LevelDB >12, WiscKey → ~1 a
  partir de 1 KB (p. 9).
- Point lookup 100 k ops em DB 100 GB random (Fig. 11): WiscKey **12×**
  LevelDB em 1 KB (p. 9).
- Range 4 GB de um DB 100 GB (Fig. 12): em random-fill + value 64 B,
  WiscKey é **12× pior** que LevelDB; em values grandes chega a 8.4×
  melhor. Sequential-fill 64 B: WiscKey 25% mais lento (lê key+value do
  vLog); values grandes 2.8× melhor (p. 10).
- GC em background no random-load, value 4 KB (Fig. 13): se o chunk é
  100% inválido, −10% throughput; senão ~−35%; ainda ≥70× LevelDB (p. 10).
- Crash: ALICE em ext4/xfs/btrfs, “more than 3000” crashes, “does not
  report any consistency vulnerability introduced by WiscKey” (p. 10).
  Worst-case recover (1 KB): LevelDB 0.7 s, WiscKey 2.6 s (p. 10–11).
- Space amp (Fig. 14): WiscKey **maior** enquanto o workload corre
  (invalidos + metadata); depois do GC aproxima o tamanho lógico se o
  header é pequeno vs value. Explicitam o triângulo R/W/espaço: “No
  key-value store can minimize read amplification, write amplification,
  and space amplification at the same time” (p. 11).
- CPU (Table 1, value 1 KB): range query WiscKey 30.1% vs LevelDB 11.2%
  (32 threads). CPU não é bottleneck no setup; LevelDB é single-writer
  (p. 11).
- YCSB 100 GB (Fig. 15): WiscKey mais rápido que LevelDB e RocksDB nas
  seis cargas, 1 KB e 16 KB. Load 1 KB ≥50× (usual) / ≥45× (GC always-on);
  16 KB load 104× no pior caso (p. 12). Workload-E (ranges 1–100 KVs)
  ainda favorece WiscKey mesmo a 1 KB porque cada range começa com um
  point lookup (p. 12).

**§5 Related (p. 12–13).** Distingue-se de FAWN/FlashStore/SkimpyStash/
BufferHash/SILT (hash, sem range/snapshot). VT-tree evita re-sort de
runs já sorted; WiscKey separa values independentemente da distribuição
de keys. Walnut/IndexFS/Purity usam técnicas “similar”; o paper reivindica
tratar o problema de forma “more generic and complete” (p. 13).

**§6 Conclusions (p. 13).** Separação K/V + I/O SSD-conscious; esperam
inspirar a geração seguinte.

## Tese central e argumento

A tese, nas palavras do paper:

> “The central idea behind WiscKey is the separation of keys and values
> [42]; only keys are kept sorted in the LSM-tree, while values are stored
> separately in a log.” (p. 1 / FAST 133)

O argumento: compact LSM só precisa de ordem nas **keys**. Em SSD o custo
de reescrever values (WA 10× por nível, ~50× no caminho L0–L6) já não se
paga com o gap random/sequential do HDD. Separar reduz WA efetivo para
~1 quando value ≫ key (fórmula p. 5), encolhe a LSM até caber em RAM, e
deixa o random read paralelo do SSD pagar o scan. O preço explícito é
espaço (GC atrasado), scan de values pequenos em log não-ordenado, e um
protocolo de crash/GC que o LevelDB não tinha.

A citação [42] é AlphaSort (Nyberg/Gray 1994) — o paper **não** inventa
“separar chave de payload”; inventa o empacotamento LSM+vLog+GC+ALICE
em cima de LevelDB.

## Estrutura do texto

| § | Início (p. interna / FAST) | Conteúdo |
|---|----------------------------|----------|
| Abstract | 1 / 133 | claims 2.5–111× load, 1.6–14× lookup, YCSB all six |
| 1 | 1 / 133 | motivação HDD→SSD, anúncio WiscKey |
| 2.1–2.2 | 2 / 134 | LSM + LevelDB |
| 2.3 | 3 / 135 | WA/RA; Fig. 2 |
| 2.4 | 4 / 136 | SSD; Fig. 3 |
| 3.1–3.2 | 4–5 / 136–137 | goals + separação; Fig. 4 |
| 3.3 | 6–7 / 138–139 | scan //, GC, crash; Fig. 5 |
| 3.4–3.5 | 7–8 / 139–140 | buffer, drop WAL, impl |
| 4.1 | 8–11 / 140–143 | micro; Figs. 7–14, Table 1 |
| 4.2 | 12 / 144 | YCSB; Fig. 15 |
| 5–6 | 12–13 / 144–145 | related + close |
| Refs | 14–16 / 146–148 | [1]–[54] |

## Conceitos-chave

- **Key-value separation:** keys (+ pointer) na LSM; payload noutro log.
  Não é hash-index (FAWN); mantém range e snapshot da LSM (§3.2, p. 5).
- **vLog:** ficheiro append-only de values; depois do §3.3.2, records
  `(key size, value size, key, value)` com head/tail persistidos **dentro**
  da LSM como keys especiais `'head'` / `'tail'` (p. 6–8).
- **Write/read/space amplification:** definidos em §2.3 (p. 3) e §4.1.5
  (p. 11). O paper trata os três como um triângulo — não um único mínimo.
- **L0 slowdown:** LevelDB abranda o writer se L0 > 8 ficheiros (p. 3) —
  WiscKey reduz a pressão porque a compact mexe em pouca massa.
- **Prefix-append crash property:** após crash, o ficheiro ganha um
  prefixo do append, nunca um subset não-prefixo (p. 7; [45]).
- **Drop LSM WAL:** correto *só* porque o vLog já duplica a key para GC
  (p. 8). Sem esse header, a otimização não se segura.

## Citações relevantes

1. "the same data is read and written multiple times throughout its lifetime; as we show later (§2), this I/O amplification in typical LSM-trees can reach a factor of 50x or higher" (p. 1 / FAST 133)
2. "The central idea behind WiscKey is the separation of keys and values [42]; only keys are kept sorted in the LSM-tree, while values are stored separately in a log." (p. 1)
3. "We decouple key sorting and garbage collection in WiscKey while LevelDB bundles them together." (p. 1)
4. "if small values are written in random order, and a large dataset is range-queried sequentially, WiscKey performs worse than LevelDB." (p. 2)
5. "For a large dataset, since any newly generated table file can eventually migrate from L0 to L6 through a series of compaction steps, write amplification can be over 50 (10 for each gap between L1 to L6)." (p. 3)
6. "Therefore, considering the 14 SSTable files in the worst case, the read amplification of LevelDB is 24 × 14 = 336." (p. 3)
7. "Compaction only needs to sort keys, while values can be managed separately [42]." (p. 5)
8. "assuming a 16-B key, a 1 KB value, and a write amplification of 10 for keys (in the LSM-tree) and 1 for values, the effective write amplification of WiscKey is only (10 × 16 + 1024) / (16 + 1024) = 1.14." (p. 5)
9. "Deleting a key simply deletes it from the LSM tree, without touching the vLog." (p. 5)
10. "the tuple (key size, value size, key, value) is stored in the vLog." (p. 6)
11. "only some prefix of the appended bytes will be added to the end of the file during file-system recovery [45]. It is not possible for random bytes or a non-prefix subset of the appended bytes to be added to the file." (p. 7)
12. "WiscKey implements synchronous inserts by flushing the vLog before performing a synchronous insert into its LSM-tree." (p. 7)
13. "WiscKey’s throughput is 46× and 111× of LevelDB for the 1-KB and 4-KB value size respectively." (p. 9; random-load 100 GB)
14. "WiscKey performs 12× worse than LevelDB for 64-B key-value pairs due to the device’s limited parallel random-read throughput for small request sizes" (p. 10; range, random-filled DB)
15. "ALICE checks more than 3000 selectively-chosen system crashes, and does not report any consistency vulnerability introduced by WiscKey." (p. 10)
16. "No key-value store can minimize read amplification, write amplification, and space amplification at the same time." (p. 11)
17. "during load, for 1-KB values, WiscKey performs at least 50× faster than the other databases in the usual case, and at least 45× faster in the worst case (with garbage collection switched on always)" (p. 12)

## Diálogo teórico

No texto (não a biblio inteira):

- **O’Neil LSM [43]** — estrutura que WiscKey não abandona (§2.1, p. 2).
- **LevelDB [48] / BigTable [16]** — fork e vocabulário (L0–L6, memtable).
- **RocksDB [25]** — baseline YCSB; “optimizations are orthogonal” e ainda
  tem WA alta porque o desenho é “fundamentally similar to LevelDB” (p. 13).
- **bLSM [49]** — scheduler + bloom; citado como otimização LSM clássica
  (p. 13), não como alternativa à separação.
- **VT-tree [50]** — evita re-sort de runs already-sorted; WiscKey diz que
  a separação reduz WA *independentemente* da distribuição de keys (p. 13).
- **FAWN / FlashStore / SILT [8, 22, 35]** — log + hash; WiscKey recusa
  perder range/snapshot (p. 12–13).
- **Walnut / IndexFS / Purity [18, 47, 19]** — “similar techniques”;
  reivindica generalidade (p. 13).
- **Nyberg/Gray AlphaSort [42]** — ancestral da separação key/payload (p. 1, 5).
- **Pillai et al. ALICE [45]** — propriedade de append e a ferramenta de
  crash (§3.3.3, §4.1.4).
- **YCSB [21]** — macro que o paper trata como “real-world use cases” (p. 2, 12).

## Relação com Pedra

- **Crate / RFC:** `pedradb-core` `vlog.rs` + `Db::compact_vlog`;
  RFC-0014 P2.2 (spill); RFC-0016 P0.1 (rewrite GC).
- **Ledger:**
  - **L4 `SHIP` confirmado pela ficha.** Threshold spill é um *recorte*
    do WiscKey: o paper mete **todos** os values no vLog; Pedra só os
    acima de `large_value_threshold`. Isso é coerente com o próprio
    paper — o ganho de WA colapsa quando value ≲ key (Fig. 9–12, 64 B).
  - **L5 precisa de cisão.** `compact_vlog` já existe: rewrite de live
    records para `VALUES.vlog.new` + adopt no MANIFEST. Isso **não** é o
    GC online head/tail + hole-punch + `'tail'` na LSM do §3.3.2. O que
    continua `OPEN` é GC **incremental / em background / sem reescrever
    o log inteiro**. Não implementar o drop do WAL da LSM (§3.4.2): o
    Pedra precisa do WAL para TX multi-key e para values *inline*; o
    vLog do Pedra **não** guarda a key em cada record (layout
    `len|crc|data`, sem o tuple do Fig. 5).
- **Não copiar sem medida:** pool de 32 threads no `scan` (§3.3.1);
  `posix_fadvise`; depender da propriedade de prefix-append do FS sem
  CRC no record (nós temos CRC — melhor).
- **Teste se roubássemos o GC incremental:** crash entre fsync do vLog e
  persistir o novo tail (o paper ordena: append → fsync vLog → sync
  LSM(tail+addrs) → punch). DST: `FailingEnv` nesse ponto não pode
  deixar pointer na LSM para offset punched, nem tail atrás de data
  viva. Já temos testes de mid-GC no rewrite (`compact_vlog_mid_gc_*`).
- **O que falsifica o claim no Pedra:** (1) workload de values pequenos
  no vLog (o paper perde 12× no range 64 B, p. 10); (2) scan longo sobre
  log random-filled sem prefetch; (3) HDD / device sem paralelismo
  (todo o §2.4); (4) comparar contra Rocks com compressão e tuning, não
  contra LevelDB 1.18 sem compressão em 840 EVO 2016.

Fichas irmãs: R006 Monkey (D4). Ainda inexistentes: R018 HashKV (GC
local), R010 Rocks Experience, R088 Pillai (a propriedade de FS).

## Avaliação crítica

- **Hardware 2016:** um SATA consumer SSD + 64 GB RAM. Os 46×/111× e a
  LSM de 2 GB “cabe na RAM” **não** se transportam para NVMe moderno nem
  para datasets em que o índice de keys não cabe. Fig. 3 é o chão do
  argumento de scan paralelo.
- **Baseline:** LevelDB 1.18, compressão off, `db_bench` uniforme.
  RocksDB entra só no YCSB e “default configuration”. Não é Pebble, não
  é Rocks 2024, não há BlobDB/Titan no gráfico.
- **WA “50×”** é o pior caso analítico L1–L6 × 10; a medição da Fig. 2
  no load 100 GB é **14**, não 50. O abstract e a intro usam o pior caso.
- **YCSB “all six”** inclui o pior WiscKey (E) em ranges *curtos* (1–100),
  que o paper admite serem o caso favorável (cada range ≈ point lookup).
  O pior caso que eles próprios documentam (scan de 4 GB em random-fill
  64 B) **não** está no YCSB da Fig. 15.
- **Crash:** ALICE no *Put* path; não substitui DST de GC incremental
  (punch + tail). Recover 2.6 s vs 0.7 s é o custo de scan do vLog desde
  o último head.
- **Espaço:** o paper é honesto — WiscKey compra WA/RA com space amp
  até o GC correr. Um `compact_vlog` full-rewrite (Pedra) inverte esse
  trade: paga I/O de GC quando o operador pede, em vez de continuamente.
- **Não medem:** multi-writer, compressão, value sizes mistos no mesmo
  DB, geo/replica, TX multi-key (LevelDB batch ≠ Pedra OCC).

## Palavras-chave

lsm; wisckey; value-log; write-amplification; read-amplification;
space-amplification; garbage-collection; ssd; leveldb; range-query;
crash-consistency; alice

## Fontes

**Primária:** `docs/references/wisckey-fast2016.pdf` (FAST ’16, open
USENIX) + `docs/references/wisckey-fast2016.txt`.

**Secundárias (não citadas como se fossem este paper):** RFC-0014 P2.2;
RFC-0016 P0.1; `crates/pedradb-core/src/vlog.rs` (layout Pedra, para a
seção Relação). HashKV / Titan / BlobDB **não** foram lidos nesta ficha.
