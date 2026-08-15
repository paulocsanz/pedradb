# TIMELINE — research/

Registro **append-only**. Uma entrada por sessão que tocou o catálogo.
Não reescrever o passado; corrigir em entrada nova.

---

## 2026-08-14 — pasta + 100 + metodologia coop

- Criado `research/` com catálogo ranqueado de 100 papers (engine /
  dist / layer / HTAP / TX / test), `catalog.tsv`, backlog, queue.
- 13 PDFs já em `docs/references/` marcados `have-pdf`. Zero fichas.
- Importada a metodologia de `../coop` (D0–D3, fonte no disco, TIMELINE,
  skill + score, Ctrl+F ≠ lido, “vai” ≠ throughput) e o **ledger** de
  `../determinismo/pedradb-dst` (tentou / vale / recusa / bloqueado).
- Estrutura: `QUALIDADE.md`, `AGENTS.md`, `fichamentos/`, `sinopses/`,
  `fontes/`, `correlacao/`, `LEDGER.md`, `PLANO.md`.
- Skill `.grok/skills/audit-qualidade` + `score_fichamento.py`.
- **Não feito:** nenhuma ficha D3; nenhum PDF novo baixado.

**Pendente (vai):** Wave 0 de `PLANO.md` — próximo: R006 Monkey.

---

## 2026-08-14 — ficha R005 WiscKey (D4)

- Lido `docs/references/wisckey-fast2016.pdf` na íntegra (§§1–6).
- `fichamentos/ficha_R005_Lu_WiscKey.md` — score `D4 328L citas=17`.
- Sinopse escrita. `catalog.tsv` / CATALOG / bibliografia → `ficha` / ✅.
- Ledger: L4 `src=ficha`; L5a rewrite `SHIP` (já era RFC-0016); L5
  incremental continua `OPEN`; **L5b `REFUSE`** dropar WAL da LSM
  (vLog Pedra não carrega a key no record — §3.4.2 não se aplica).
- RFC-0014 ainda diz “GC deferred”; o código tem `compact_vlog`. Não
  editei o RFC (é texto histórico P2.2). A cisão vive no LEDGER.

**Pendente (vai):** R006 Monkey (ficha) *ou* 0026 P0.1 se a frente for value-store.

---

## 2026-08-14 — RFCs hipotéticas do value store (0026–0029)

- Pedido: aprofundar GC incremental e caminhos melhores, RFC para o Paulo escolher.
- HashKV ATC'18 persistido: `docs/references/hashkv-atc2018.pdf` (+ `.txt`). Lido §1–3.3 + Fig. 2. **Não** é ficha D3.
- Achado que muda o menu: vLog circular no *Update* Zipf tem WA **19.7×** (HashKV Fig. 2) — pior que Rocks 7.9×. Incremental estilo WiscKey (0027) não é “o upgrade óbvio”.
- RFCs draft: [0026 menu](../docs/rfc/0026-value-store-evolution-menu.md), [0027 tail](../docs/rfc/0027-incremental-vlog-gc.md), [0028 hash](../docs/rfc/0028-hash-partitioned-value-store.md), [0029 blobs+prefetch](../docs/rfc/0029-blob-generations-and-scan-prefetch.md).
- Ledger L17–L20 `MEASURE`. Correlação `correlacao/02_value_store_gc.md`.
- **Nada implementado.**

**Pendente:** Paulo escolhe A/B/C/ficar no rewrite; Wave 0 de fichas continua no R006.

---

## 2026-08-14 — RFC-0026 P0 (medida + pick C)

- `DbStats::vlog_line` / `vlog_live_ratio`; `pedra stats <db>`; `usage.md`.
- Bench `vlog_zipf`: 2000×4 KiB, Zipf 0.99, 3 passes. Ratio após update+`latest_only` = **0.499**. `compact_vlog` 86–168 ms (16→8 MiB). Update 12–22 s. JSON em `findings/rfc0026-vlog-zipf/`.
- **Não** reproduziu HashKV 19.7× (escala 2000× menor, métrica diferente).
- P0.3: **C** (0029). A recusada. B só se C ainda doer em GB. Rewrite fica até 0029 P0.
- Teste `stats_amp_and_vlog_metrics` atualizado (ratio só cai depois de `latest_only`).

**Pendente (vai):** 0029 P0 *ou* ficha R006.

---

## 2026-08-14 — RFC-0029 P0 (blobs + prefetch)

- `VLG3` + `000001.blob`; `Db::set_vlog_rotate_bytes`; `compact_blob` recusa o ficheiro ativo; prefetch N=4 single-thread.
- Bug na 1ª corrida: `get` → None depois de rodar. Causa: `read_ptr_on` usava `self.path` para file 0; depois da rotação o handle é o `.blob`. Corrigido (`path_for_ptr`). Rotação começa em gen 1. `remap_stored_value` não toca em VLG3 (offset 8 colide).
- Testes: `blob_rotate_reopen`, `blob_rotate_keeps_legacy_vlg1`, `compact_blob_drops_dead_only`, `compact_blob_crash_after_new_file_keeps_reads`, `scan_prefetch_same_visible_kvs`, `read_ptr_on_file0_after_blob_handle`.
- **Não** feito: MANIFEST lista blobs (descoberta por nome); cap não persiste no open; auto worst-ratio (P1.1); HashKV D3; 0027/0028.

**Pendente (vai):** ficha R006 Monkey *ou* 0029 P1.1 auto-pick.

---

## 2026-08-14 — ficha R006 Monkey (D4)

- Lido `docs/references/monkey-sigmod2017.pdf` na íntegra (§§1–7 + Apps A–E; Fig. 12 só caption). Extração paginada `monkey-sigmod2017.pages.txt`.
- `fichamentos/ficha_R006_Dayan_Monkey.md` + sinopse. Eq. 3 no `.txt` sem páginas estava errada (duas ramas *tiering*); PDF p. 6: *leveling* \(R=\sum p_i\).
- L1 `SHIP` (src=ficha): Bloom-por-SST é o SOTA de que o paper parte. L2 **continua `MEASURE`**: 50–80% é HDD 7200 RPM + cache off + zero-result; Pedra tem 10 bits/key e `MAX_LSM_LEVEL=3`. Navigable Monkey (\(T\)+tiering) ≠ alocação de Bloom.
- WiscKey (p. 12) compatível mas o modelo de \(R\) não conta o I/O do vlog — não reabre L4/L5.
- **Não** implementado: `bits_per_key` por nível.

**Pendente (vai):** ficha R007 Dostoevsky.

---

## 2026-08-14 — ficha R007 Dostoevsky (D4)

- Lido `docs/references/dostoevsky-sigmod2018.pdf` (§§1–7 + Apps A–G início; H/I só Fig. 11). Extração `dostoevsky-sigmod2018.pages.txt`.
- Tese: merges em 1…L−1 são superfluous *se* o Bloom for Monkey. Lazy Leveling = tiering em cima + leveling em L. Fluid (K,Z). Dostoevsky navega.
- L3 **REFUSE** confirmado (`src=ficha`): (1) exige L2; (2) short range piora — Pedra tem `scan`; (3) `MAX_LSM_LEVEL=3`; (4) HDD RAID; (5) “dominates” é o navegador pré-sintonizado (App. F), não LL default. Reabrir só com o gate da ficha.
- **Não** implementado.

**Pendente (vai):** ficha R041 Percolator.

---

## 2026-08-14 — ficha R041 Percolator (D4)

- Lido `docs/references/percolator-osdi2010.pdf` na íntegra (§§1–5). Extração `percolator-osdi2010.pages.txt`.
- SI + 2PC cliente + primary lock + oracle sobre Bigtable (Figs. 4–6). Observers ≠ triggers (outra TX). Caffeine: 100× / −50% vs MR; write 0.23× Bigtable; TPC-E-like 2–5 s, ~30× CPU.
- L22 `REFUSE` copiar o protocolo (kernel OCC+WAL; store intents FDB). L23 `REFUSE` oracle-serviço. OCC do kernel valida read-set — mais forte que SI (write skew).
- **Não** implementado.

**Pendente (vai):** ficha R048 TiDB.

---

## 2026-08-15 — ficha R048 TiDB (D4)

- Lido `docs/references/tidb-raft-htap-vldb2020.pdf` (§§1–8). Extração `tidb-raft-htap-vldb2020.pages.txt`.
- Learner ≠ quorum ≠ eleição (p. 2). TiFlash **read-index no leader** (§4.2.4) — não é LocalApplied. Freshness ≈ 1 s (Table 4). Isolation ≤10% TPS (Fig. 10) vs MemSQL >5×.
- L8 `SHIP` confirmado (src=ficha). **L24 `REFUSE`** SI/read-index no fold. TiFlash ≠ fold (papel Raft sim; formato e read não). L7/L22/L23 intactos.
- Wave 0 chega a **5 fichas D4**.
- **Não** implementado.

**Pendente (vai):** fetch + ficha R010 Rocks Experience.

---

## 2026-08-15 — ficha R010 Rocks Experience (D4)

- Fetch USENIX: `research/fontes/R010_Dong_2021_RocksExperience.pdf` (+ `.txt` / `.pages.txt`).
- Lido §§1–9 + Apps A–C. Tese: library de um nó; alvo WA → espaço → CPU; “most workloads are space constrained”.
- L9 `SHIP` src=ficha. Espaço>WA confirma 0026-C. BlobDB confirma L4. **L25/L26/L27 MEASURE** (WAL-skip só com Raft log; file checksum; user-ts). L5b intacto.
- **Não** implementado.

**Pendente (vai):** fetch + ficha R043 FoundationDB.

---

## 2026-08-15 — ficha R043 FoundationDB (D4)

- Fetch `foundationdb.org/files/fdb-paper.pdf` → `research/fontes/R043_Zhou_2021_FoundationDB.pdf`.
- Lido §§1–8. Unbundled TS/LS/SS; OCC+MVCC strict serializable; simulação primeiro; f+1 + recovery ~3 s; janela 5 s.
- L9 confirmado. **L28 MEASURE** swarm/buggify. **L29 REFUSE** clonar o TS. **L30 REFUSE** 5 s no kernel. Não implementado.

**Pendente (vai):** fetch + ficha R044 Record Layer.

---

## 2026-08-15 — ficha R044 Record Layer (D4)

- Fetch `foundationdb.org/files/record-layer-paper.pdf` →
  `research/fontes/R044_Chrysafis_2019_RecordLayer.pdf`.
- Lido §§1–12 + Apps. A–C. Layer stateless; record store;
  índices na mesma TX; CloudKit Table 1 (Solr-eventual → TX);
  VERSION+incarnation; continuations por 5 s; SQL *acima* do RL.
- L9 confirmado. **L31 SHIP** same-TX indexes. **L32 REFUSE**
  produto RL no kernel. **L33 MEASURE** VERSION no fold.
  **L34 REFUSE** atomics FDB no kernel. Não implementado.

**Pendente (vai):** fetch + ficha R012 compaction design space.

---

## 2026-08-15 — ficha R012 LSM compaction design space (D4)

- Fetch `vldb.org/pvldb/vol14/p2216-sarkar.pdf` →
  `research/fontes/R012_Sarkar_2021_LSMCompaction.pdf`.
- Lido §§1–7. Quatro primitivas; 10 strategies no Rocks 6.11.4;
  Full 63×; partial −34–56%; Tier tail ~25 ms; 1-Lvl mais estável.
- Pedra `compact_levels` = Full do par. **L35 MEASURE** LO+1.
  **L36 REFUSE** menu. **L37 REFUSE** universal default.
  **L38 MEASURE** TSD/TSA com SLA. L3 intacto. Não implementado.

**Pendente (vai):** fetch + ficha R045 CockroachDB 2020.

---

## 2026-08-15 — ficha R045 CockroachDB (D4)

- Fetch cockroachlabs PDF → `research/fontes/R045_Taft_2020_CockroachDB.pdf`.
- Lido §§1–9. Multi-Raft Ranges 64 MiB; leaseholder; Parallel Commits;
  HLC 500 ms (não strict SI); TPC-C 1.25 M tpmC / 8 TB. Rocks black box
  (Pebble **não** medido). Wave 0 fecha em **10 fichas D4**.
- L9/L29 confirmados. **L39 MEASURE** split/lease quando >1 grupo.
  **L40 REFUSE** protocolo TX CRDB no kernel. L24 intacto. Não implementado.

**Pendente (vai):** ficha R018 HashKV (PDF já em `docs/references/`).

---

## 2026-08-15 — ficha R018 HashKV (D4)

- Lido `docs/references/hashkv-atc2018.pdf` na íntegra (§§1–6, Exp 1–9).
  Extração `hashkv-atc2018.pages.txt`.
- Fig. 2 Update WA 19.7× (vLog) vs Rocks 7.9×. HashKV: grupos por
  hash(key); GC sem get LSM; 3.7–4.6× vLog no eval (RAID-0, cache on,
  crash off). 256 B perde; P0 hash 7.9% mais lento que vLog.
- L19 `MEASURE` src=ficha — 0028 P0 **não** começa (0026-C). L4/L5b
  confirmados. L5: tail é o desenho da Fig. 2. Não implementado.

**Pendente (vai):** fetch + ficha R013 Spooky.

---

## 2026-08-15 — ficha R013 Spooky (D4)

- Fetch `vldb.org/pvldb/vol15/p3071-dayan.pdf` →
  `research/fontes/R013_Dayan_2022_Spooky.pdf`.
- Lido §§1–8. Full ≤50% utilização; Partial WA = compact × GC SSD.
  Spooky alinha fronteiras ao \(L\); \(X=L-2\); 644 vs 369 GB; total
  WA ≈2.5× melhor que Partial no NVMe cheio.
- L11 `MEASURE` src=ficha — **depois** de L35; \(L\le 3\) hoje não é
  o Full do paper. Não implementado.

**Pendente (vai):** fetch + ficha R014 Endure.

---

## 2026-08-15 — ficha R014 Endure (D4)

- Fetch `vldb.org/pvldb/vol15/p1605-huynh.pdf` →
  `research/fontes/R014_Huynh_2022_Endure.pdf`.
- Lido §§1–10. Nominal sobre-ajusta; robusto = pior caso na bola KL
  (T, bits, L|T). Modelo 5×; Rocks 2.4× / 90% I/O. Table 3: robusto
  **sempre Leveling**. Retune online inviável.
- L12 `MEASURE` src=ficha — só com ≥2 knobs; **não** tuner. L3/L37
  confirmados. L2/L36 intactos. Não implementado.

**Pendente (vai):** fetch + ficha R017 SILK.

---

## 2026-08-15 — ficha R017 SILK (D4)

- Fetch USENIX ATC'19 → `research/fontes/R017_Balmau_2019_SILK.pdf`.
- Lido §§1–8. p99 = interferência write/flush/compact. 3 lições: Cm
  cheia; rate-limit adia e agrava; menos compact (TRIAD/Pebbles) adia.
  Nutanix: 2–3 ordens no p99; 0 stall. YCSB −≤7% throughput.
- L41 `MEASURE` nomes de stall + flush>L0. Não scheduler completo.
  WAL off no eval — p99 Pedra sync é outro eixo. Não implementado.

**Pendente (vai):** fetch + ficha R016 ADOC.

---

## 2026-08-15 — ficha R016 ADOC (D4)

- Fetch USENIX FAST'23 → `research/fontes/R016_Yu_2023_ADOC.pdf`.
- Lido §§1–8. Stall = data overflow (MMO/L0O/RDO = MT/L0/PS).
  Table 2: teses CPU/BW/L0/fundo não generalizam. Tuner AIMD
  threads+batch, Tw=1 s, ~300 LOC. Fig. 12 +66.7% vs SILK-O no
  PM; p99 pior em 3/4 discos. 87.9% só no abstract.
- L41 ganha taxonomia. L42 `REFUSE` tuner (0 knobs; DST-hostil).
  SILK complementar, não substituído. Não implementado.

**Pendente (vai):** fetch + ficha R031 Lethe.

---

## 2026-08-15 — ficha R031 Lethe (D4)

- Fetch autor BU → `research/fontes/R031_Sarkar_2020_Lethe.pdf`.
- Lido §§1–7. Persistência unbounded; full-tree = anti-padrão
  (X-Engine). FADE = TTL exponencial + SO/SD/DD. KiWi =
  delete tiles (h=1 = clássico). §5.1: space −48%, lookup
  +17%, WA 1.4× → +0.7%. Abstract 1.4×/9.8× não reaparece.
- L38 `MEASURE` FADE depois de L35, só com SLA Dth.
  L43 `REFUSE` KiWi. Não implementado.

**Pendente (vai):** fetch + ficha R023 Disco.
