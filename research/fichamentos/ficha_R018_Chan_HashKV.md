# Fichamento: R018 — HashKV (ATC’18)

**Status:** D4
**Lido em:** 2026-08-15
**Catalog:** [CATALOG.md](../CATALOG.md) · `R018`

## Referência

CHAN, Helen H. W.; LI, Yongkun; LEE, Patrick P. C.; XU,
Yinlong. **HashKV: Enabling Efficient Updates in KV Storage
via Hashing.** In: *2018 USENIX Annual Technical Conference
(ATC ’18)*, 11–13 July 2018, Boston, MA. 13 pp. (corpo
1007–1017 + refs 1018–1019).
URL: https://www.usenix.org/system/files/conference/atc18/atc18-chan.pdf
Código: http://adslab.cse.cuhk.edu.hk/software/hashkv

Locus = **p. USENIX** (1007–1019). Figs. 1–12 e Table 1
conferidos no texto extraído. Relatório técnico [5] **não**
lido (espaço / scans pós-update extra).

## Dados da leitura

- **Estrato:** (a) fonte primária lida na íntegra: abstract,
  §§1–6, Figs. 1–12, Table 1. Referências [1]–[43] **não**
  foram relidas. WiscKey [23] só pelo que *este* texto
  afirma (R005 já D4). PebblesDB [30] listed. Tech report
  [5] fora.
- **Arquivo lido:** `docs/references/hashkv-atc2018.pdf` +
  `.txt` + `.pages.txt`.
- Como foi lido: PDF local, seção a seção, nesta sessão.
  Sessão anterior tinha só §1–3.3 + Fig. 2 — **não** conta
  como ficha; o eval §§4.2–4.5 e related entram agora.
- **Língua original:** EN. Sem tradução.
- **Tier: D4** — via **D4-b** (PDF original EN)

## Resumo / Síntese

**Abstract / §1 (p. 1007).** LSM sofre WA ≥50× e RA
\>300× (citam WiscKey/LSM-trie). KV-separation (WiscKey)
mete keys na LSM e values num log circular — reduz a
árvore, mas **update-intensive** paga GC caro. Duas
razões: (1) GC *tem* de começar no tail (relocates
cold-valid); (2) cada record no tail exige **get na LSM**
para saber se está vivo. HashKV: hash da key → partição
fixa; updates e GC determinísticos. Extensões para
relaxar o mapa fixo. Claim vs “current KV separation”:
**4.6×** throughput e **53.4%** menos write traffic.

**§2 Motivação (p. 1008–1009).** Recapitula LevelDB
(Fig. 1): MemTable → L0 2 MiB SST; compact Li→Li+1
lê 10 SST no pior caso. **§2.2** WiscKey/vLog: head
append, tail GC, query LSM, key+meta *junto* do value,
over-provision. Limitação: workloads reais são Zipf-hot
[3]; tail é muitas vezes frio-válido. Segmentar o vLog
e escolher o “melhor” segmento (cost-benefit) ainda
mistura hot/cold. Hot/cold em duas regiões *sem* hash
obriga get na LSM em cada update.

**Fig. 2 / protótipo vLog (p. 1009).** Load 40 GiB ×
1 KiB; Update 40 GiB Zipf **0.99**; 40 GiB + **30%**
(12 GiB) reserve; write cache **desligado**. Load:
vLog WA **1.6×**. Update: vLog **19.7×**, LevelDB
**19.1×**, RocksDB **7.9×**. Este é o número que
0026/0027 já usaram — agora com o resto do paper.

**§3 Design (p. 1010–1012).** PUT/GET/DELETE/SCAN.
Value store ≠ hash da LSM (LSM única, para SCAN).

- **Main segments** 64 MiB + **log segments** 1 MiB
  no reserved. `hash(key)` → main; se cheio, aloca
  log segs. Main + logs = **segment group**. Segment
  table em RAM (fim de cada grupo + lista de logs) +
  checkpoint. Write path **sem** get LSM.
- Record no value store = key + meta + value (como
  WiscKey). Pedra hoje é `len|crc|data` — sem key.
- **Write cache** opcional, in-place; “degrading
  reliability”; pode desligar (§3.2). Fig. 2 desliga;
  o eval default **liga** 64 MiB (§4.1).
- **GC (§3.3):** dispara quando acabam log segs
  livres. Greedy = grupo com **mais writes** (heap
  na segment table). Scan do grupo **sem** LSM:
  última escrita da key = viva (log-structured).
  Hash table temporária por grupo, limitada ao
  grupo. Depois reescreve o main (+ logs se
  preciso), solta logs, actualiza ponteiros na LSM.
- **Hotness (§3.4):** no GC, “hot” = actualizado
  ≥1 vez desde o insert. Hot volta ao grupo; cold
  vai a um **cold data log** (GC estilo vLog) e o
  grupo fica só com meta+tag. Update posterior lê
  a tag *sem* LSM e trata como hot. Cold log pode
  ir para HDD.
- **Selective sep (§3.5):** values pequenos ficam
  na LSM. Threshold = teste no deployment — não
  dão um número canónico.
- **SCAN (§3.6):** values espalhados → random I/O.
  Mitigam com `posix_fadvise` read-ahead (igual
  WiscKey). Sem isto, 256 B scan **+81%** quando
  ligam (p. 1016).
- **Crash (§3.7):** write journal (flush do cache:
  values → journal → commit → LSM). **GC journal**
  porque o GC *overwrite* o grupo: (i) dump dos
  live que vão ser overwritten + meta; (ii) write
  back; (iii) LSM; (iv) free. Sem isto, crash a
  meio do GC perde live.
- **Impl (§3.8):** C++ / LevelDB **1.20**, ~6.7 KLOC.
  Value store = **um ficheiro** grande no Ext4 de
  um RAID `mdadm`; segs alinhados. 32 threads no
  flush, 8 no GC. Batch 4 KiB para esconder
  writes hash-scattered.

**§4 Eval (p. 1013–1017).** Ubuntu 14.04, Xeon
E3-1240v2, 16 GiB RAM, 6× **Plextor M5 Pro 128 GiB**
RAID (chunk 4 KiB) + 1 SSD OS. LevelDB 1.20,
RocksDB 5.8, HyperLevelDB, PebblesDB, vLog próprio.
Default: 40 GiB + 30% reserve; cache 64 MiB; async;
**hotness, selective e crash consistency OFF** salvo
quando o experimento os liga. YCSB 1 KiB (8 B meta
+ 24 B key + 992 B value). P0 load 40 GiB; P1–P3
três passes de 40 GiB Zipf 0.99 (120 GiB updates).

- **Exp 1 (p. 1014, Fig. 5).** P0: HashKV **17.1×**
  LevelDB, **3.0×** Rocks; **7.9% mais lento** que
  vLog (writes aleatórios do hash). P1–P3: HashKV
  **6.3–7.9×** LevelDB, **1.3–1.4×** Rocks,
  **3.7–4.6×** vLog. Write size −71.5% / −66.7% /
  **−49.6%** vs LDB/RDB/vLog. Tamanho do store
  semelhante. HyperLevelDB / PebblesDB ≥2× o
  throughput de HashKV, mas store **2.2× / 1.7×**
  maior (lixo).
- **Exp 2 (Fig. 6).** Reserve 10–90%: HashKV
  **3.1–4.7×** vLog; write −30.1–57.3%. Breakdown:
  vLog gasta tempo em **GC-Lookup** (LSM); HashKV
  não.
- **Exp 3 (Fig. 7).** RAID-6: HashKV ainda
  **4.8× / 3.2× / 2.7×** LDB/RDB/vLog. Write
  +20% RAID-5, +50% RAID-6 (paridade).
- **Exp 4 (Fig. 8).** 256 B: HashKV e vLog **perdem**
  para LDB/Rocks (overhead do value store). 4 KiB:
  HashKV **15.5× / 2.8×** LDB/Rocks; **2.2–5.1×**
  vLog entre 256 B e 4 KiB. 64 KiB: HashKV
  **−10.7%** vs vLog (LSM já minúscula; lookup do
  vLog barato).
- **Exp 5 (Fig. 9).** Scan 4 GiB, 1 MiB/request.
  HashKV ≈ vLog. Vs LevelDB: **−70%** (256 B),
  **−36.3%** (1 KiB); **+94.2%** a 4 KiB.
- **Exp 6 (Fig. 10).** Hotness ON, reserve 20%:
  throughput **+113.1% / +121.3%** (Zipf 0.90 /
  0.99); write **−42.8% / −42.5%**.
- **Exp 7 (Fig. 11).** Selective ON: +23.2–118%
  (large=1 KiB) / +19.2–52.1% (large=4 KiB);
  write −14.1–39.6% / −4.1–10.7%. Mais ganho
  quando há mais keys pequenas.
- **Exp 8 (Table 1).** Crash ON: P3 **58.0 → 54.3
  KOPS (−6.5%)**; write 454.6 → 473.7 GiB
  **(+4.2%)**. Correctness via “code injection and
  unexpected terminations” — **não** é DST com
  seed.
- **Exp 9 (Fig. 12).** Main 16→256 MiB, reserve
  20%: throughput **+52.5%**. Log 256 KiB→4 MiB,
  reserve 20%: throughput **−16.1%** (pior
  utilização). Cache 4→64 MiB: **+29.1%** /
  write −16.3%.

**§5–6 (p. 1017).** Relacionam bLSM, VT-Tree,
LSM-trie, LWC, SkipStore, PebblesDB (compact);
WiscKey / Atlas / Cocoytus (sep); Dynamo/Ceph/NVMKV
(hash placement). Conclusão: o novelty é o
*grouping* para GC, não um LSM novo. HashKV “can
also adopt other KV stores” — futuro.

## Tese central e argumento

KV-separation **sem** GC local devolve o WA no
update Zipf (Fig. 2: 19.7× \> Rocks 7.9×). O
remédio não é “tail mais esperto”: é **agrupar
por hash(key)** para (i) GC do grupo mais sujo
e (ii) validade = última escrita no grupo, sem
get LSM (p. 1011). Selective sep e hot/cold são
extensões; o write cache e o RAID-0 são o
*banco de ensaio*, não a tese.

O paper **não** diz para Pedra largar blobs
(0029) nem o rewrite. Diz que o próximo GC,
*se* o update-heavy grande voltar a doer, não
é o tail do WiscKey.

## Estrutura do texto

| § | p. | Conteúdo |
|---|---:|----------|
| Abstract / 1 | 1007 | 4.6× / −53.4%; tese GC |
| 2.1 LevelDB | 1008 | Fig. 1; WA 50× / RA 300× |
| 2.2 vLog | 1009 | Fig. 2: 1.6× / **19.7×** |
| 3.1–3.2 | 1010 | grupos; cache opcional |
| 3.3–3.5 | 1011 | GC sem LSM; tag; threshold |
| 3.6–3.8 | 1012 | scan fadvise; journals; RAID |
| 4.1–4.2 | 1013 | setup; Exp 1–2 |
| 4.3–4.5 | 1015 | size; scan; hot; crash |
| 5–6 | 1017 | related; conclusão |
| refs | 1018 | [1]–[43] |

## Conceitos-chave

- **Segment group.** Main 64 MiB + log segs 1 MiB
  do mesmo `hash(key)` (p. 1010). Termo do paper.
- **Deterministic grouping.** Todas as versões de
  uma key no mesmo grupo; write sem get LSM.
- **Greedy GC.** Grupo com mais bytes escritos
  (p. 1011).
- **Validity without LSM.** Última append da key
  no grupo = live.
- **GC journal.** Preciso porque o GC *overwrite*
  live (p. 1012). ≠ punch de tail.
- **Tag / cold data log.** Hotness só no GC, não
  no write path (p. 1011).
- **Selective KV separation.** Pequenos na LSM
  (p. 1012). Pedra já tem threshold (L4).
- **Write cache.** Opcional; “degrading
  reliability” (p. 1010).

## Citações relevantes

1. "HashKV achieves 4.6× throughput and 53.4% less write traffic compared to the current KV separation design." (p. 1007)
2. "the circular log maintains a strict GC order, as it always performs GC at the beginning of the log where the least recently written KV pairs are located." (p. 1007)
3. "the GC operation needs to query the LSM-tree to check the validity of each KV pair." (p. 1007)
4. "in the Update phase, vLog has a write amplification of 19.7×, which is close to LevelDB (19.1×) and higher than RocksDB (7.9×)." (p. 1009, Fig. 2)
5. "HashKV maintains a single LSM-tree for indexing (instead of hash-partitioning the LSM-tree as in the value store) to preserve the ordering of keys and the range scan performance." (p. 1010)
6. "the write cache is an optional component and can be disabled for reliability concerns." (p. 1010)
7. "It currently adopts a greedy approach and selects the segment group with the largest amount of writes." (p. 1011)
8. "HashKV sequentially scans the KV pairs in the segment group without querying the LSM-tree" (p. 1011)
9. "the version that is nearest to the end of the segment group must be the latest one" (p. 1011)
10. "we treat the KV pairs that are updated at least once since their last inserts as hot, or cold otherwise" (p. 1011)
11. "Handling crash consistency in GC operations is different, as they may overwrite existing valid KV pairs." (p. 1012)
12. "We currently deploy HashKV on a RAID array with multiple SSDs" (p. 1012)
13. "We disable selective KV separation, hotness awareness, and crash consistency in HashKV by default" (p. 1013)
14. "In the update phases, the throughput of HashKV is 6.3-7.9×, 1.3-1.4×, and 3.7-4.6× over LevelDB, RocksDB, and vLog, respectively." (p. 1014)
15. "HashKV reduces the total write sizes of LevelDB, RocksDB and vLog by 71.5%, 66.7%, and 49.6%, respectively." (p. 1014)
16. "Both HyperLevelDB and PebblesDB achieve at least twice throughput of HashKV, while … their final KV store sizes are 2.2× and 1.7× over HashKV" (p. 1014)
17. "the queries to the LSM-tree during GC incur substantial performance overhead to vLog." (p. 1014)
18. "HashKV and vLog have lower throughput than LevelDB and RocksDB when the KV pair size is 256 B" (p. 1015)
19. "For 64-KiB KV pairs, HashKV has 10.7% less throughput than vLog." (p. 1015)
20. "the range scan throughput of HashKV increases by 81.0% for 256-B KV pairs compared to without read-ahead." (p. 1016)
21. "When hotness awareness is enabled, the update throughput increases by 113.1% and 121.3%, while the write size reduces by 42.8% and 42.5%" (p. 1016)
22. "When the crash consistency mechanism is enabled, the update throughput of HashKV in Phase P3 reduces by 6.5% and the total write size increases by 4.2%" (p. 1016, Table 1)

## Diálogo teórico

- **WiscKey [23] / R005** — o vLog que eles
  reimplementam e *matam* no Update (Fig. 2).
  HashKV guarda key+meta no value store *como*
  WiscKey; muda o *mapa* e o GC.
- **LevelDB [15] / RocksDB [12]** — baselines;
  Rocks 5.8 “default parameters”.
- **PebblesDB [30] / HyperLevelDB [11]** — ganham
  throughput, perdem espaço (p. 1014).
- **Rosenblum LFS [31] / cost-benefit [26, 32]** —
  “escolher o melhor segmento” no vLog ainda mistura
  hot/cold (p. 1009).
- **SSD hot/cold [19, 27]** — inspira o tag.
- **LSM-trie [39], bLSM [33], VT-Tree [35]** —
  outro eixo (índice), não value GC.
- **NVMKV [25]** — hash no endereço; fragmenta.
  HashKV empacota log-structured *dentro* do seg.
- **YCSB [7]** — gerador; o corpo é load+update
  Zipf, não os six workloads nomeados.

## Relação com Pedra / Montanha

Pedra hoje (`vlog.rs`): threshold spill (L4);
`VLG1` um ficheiro ou `VLG3` blobs (0029 P0);
record **`len|crc|data` sem key**; GC =
`compact_vlog` / `compact_blob` rewrite (L5a);
WAL fica (L5b). 0026 P0.3 escolheu **C** (blobs)
depois de Zipf a 8 MiB (ratio 0.499; rewrite
barato). 0028 é hipotético: “Do not start P0
until 0026 P0.3 says B” — disse **C**.

| HashKV (paper) | Pedra | Ledger |
|----------------|-------|--------|
| Circular vLog Update WA 19.7× | 0026 Zipf 8 MiB **não** reproduziu 19.7× (escala 2000× menor) | **confirma o *risco*** do tail, não o número |
| Tail + get LSM por record | L5 OPEN; Pedra sem key no record | **L5**: tail *sozinho* é o desenho da Fig. 2. Incremental só com key no record (já dito em R005) |
| Hash → grupo; GC sem LSM | 0028 VLG2; **não** começado | **L19 `MEASURE` src=ficha**. Gate: workload nomeado de values grandes + updates em que rewrite/blobs tenham WA ≥ Rocks-class *e* key no record. **Não** SHIP agora (0026-C) |
| Selective sep | `large_value_threshold` | **L4 confirmado** (src+=R018 §3.5) |
| Write cache que salta WAL | default Pedra sync | **L5b confirmado.** Paper p. 1010 desliga por fiabilidade; Fig. 2 desliga |
| GC journal (overwrite) | rewrite Pedra é tmp→rename | se L19, o journal é o DST — não “punch” |
| Scan + fadvise | 0029 prefetch P0 | **L20** já cobre; não nova linha |
| RAID-0 6 SSD + 32 threads | um NVMe / um writer | writes hash-scattered **pioram** no nosso default. 0028 já avisa |
| Hot/cold tag | — | 0028 P2.1; só depois de L19 P0 |

- **Não implementar** 0028 P0 nesta sessão. Não
  dropar WAL. Não ligar write cache. Não assumir
  RAID.
- **Teste se L19:** `hash_same_key_same_seg`; GC
  do grupo sem `Db::get`; crash a meio do overwrite
  com journal (Table 1 do paper **não** chega —
  precisa `FailingEnv` + seed). Soak Zipf ≥ o
  de 0026, values ≥ threshold, WA vs
  `compact_blob`.
- **Falsificaria L19 (não fazer):** o soak 0026
  em escala continua com rewrite barato. Já é o
  estado. **Falsificaria “não fazer nunca”:**
  um named workload em que `compact_blob` /
  rewrite tenha WA ≥ 8× (Rocks Fig. 2) *e* o
  grupo greedy fique \<4× no mesmo `Env`.

Fichas irmãs: R005 WiscKey (D4); R010 BlobDB
(confirma L4, ≠ HashKV); R012 (outro eixo de
compact). Titan listed.

## Avaliação crítica

- **RAID-0 6× SATA 2014-era + cache 64 MiB +
  crash OFF** no default. Os 4.6× incluem o
  hardware que esconde o random write do hash.
  Num NVMe single-writer o sinal inverte-se
  (eles próprios: P0 HashKV **7.9% mais lento**
  que vLog).
- **vLog é o clone deles**, não o WiscKey
  original (FAST’16, outro SSD). Fig. 2 é
  *deste* clone.
- **Abstract 53.4%** vs Exp 1 **49.6%** write
  size — vizinhos, não iguais. Usar o parágrafo
  da Fig. 5 para o número do eval.
- **Pebbles/Hyper** ganham throughput e perdem
  espaço: HashKV não é o máximo de ingest, é
  o máximo de ingest *com* space parecido a
  Rocks.
- **256 B perde** para LSM inline — justifica L4,
  não um value store universal.
- **Crash = injection**, não simulação
  determinística (contrastar R043). −6.5% é o
  custo *com* journal, sem o buraco de
  correctness que o DST apanharia.
- **Não medem:** Pedra-class \(L\le 3\); blobs
  0029; compressão; multi-writer; TX OCC;
  dataset em que o índice de keys *não* cabe
  (16 GiB RAM, 40 GiB data).

## Palavras-chave

hashkv; kv-separation; vlog; garbage-collection;
segment-group; write-amplification; zipf;
wisckey; selective-separation; hot-cold;
leveldb

## Fontes

**Primária:** `docs/references/hashkv-atc2018.pdf`
(USENIX ATC ’18) + `.txt` + `.pages.txt`.

**Secundárias (não citadas como se fossem este
paper):** ficha R005; RFC-0026/0027/0028/0029;
`crates/pedradb-core/src/vlog.rs`;
`findings/rfc0026-vlog-zipf/`;
`research/correlacao/02_value_store_gc.md`
(leitura parcial anterior). Tech report [5] **não**
lido.
