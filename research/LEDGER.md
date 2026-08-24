# LEDGER — o que a literatura faz ao Pedra

**Fonte única de verdade** das decisões *depois* (ou explicitamente *antes*,
como hipótese) de uma ficha. Atualizar **na mesma sessão**. Chat / `/tmp`
não conta.

Espelho do ledger DST (`../determinismo/pedradb-dst/findings/LEDGER.md`):
tentou / vale / falhou / bloqueado / falso. Aqui o objeto é **estratégia
de paper**, não bug.

Norma: [`QUALIDADE.md`](QUALIDADE.md). Uma linha `SHIP` sem ficha D3 é
hipótese — marcar `src=listed`.

## Tags

| Tag | Significado |
|-----|-------------|
| `SHIP` | Está no código, ou a ficha D3 diz para implementar agora |
| `MEASURE` | Hipótese mensurável; precisa bench/DST nomeado |
| `REFUSE` | Não implementar (razão + paper). Recusar é resultado |
| `OPEN` | Buraco conhecido (ex. vlog GC) |
| `BLOCKED` | Sem PDF legal / paywall / hardware que não temos |
| `FALSE` | O claim não se aplica ao nosso setting (ficha explica) |
| `SUPERSEDED` | Substituído por ficha/RFC mais novo |

`src`: `ficha` · `rfc` · `listed` · `incumbent` · `bench`

## A. Decisões já no repo (antes deste catálogo)

Vêm de RFCs + PDFs em `docs/references/`. Promover/rebaixar quando a
ficha D3 correspondente existir.

| ID | Decisão | Tag | src | Papers | Onde |
|----|---------|-----|-----|--------|------|
| L1 | Bloom por SST (skip table) | `SHIP` | ficha | R006 | RFC-0014 P0.1; ficha: filtro-por-run é o SOTA de que Monkey *parte* |
| L2 | Alocação Monkey de FPR | `MEASURE` | ficha | R006 | RFC-0012; ficha p. 8/10–11 — \(O(L)\) com `MAX_LSM_LEVEL=3` e 80% em HDD cache-off; não SHIP |
| L3 | Lazy Leveling | `REFUSE` | ficha | R007 | RFC-0012; ficha p. 5–8 / Fig. 10B — exige L2; piora short range; `L≤4`; reabrir só com gate da ficha |
| L4 | Vlog threshold spill (não *todos* os values) | `SHIP` | ficha | R005, R018 | RFC-0014 P2.2; R005 Fig. 9–12; R018 §3.5 / Exp 4 (256 B perde) |
| L5a | Vlog GC full-rewrite (`compact_vlog`) | `SHIP` | rfc | R005 | RFC-0016 P0.1; **não** é o GC do paper |
| L5 | Vlog GC incremental (head/tail + punch) | `OPEN` | ficha | R005, R018 | WiscKey §3.3.2; R018 Fig. 2: *este* desenho dá 19.7×. Pedra sem key no record |
| L5b | Dropar WAL da LSM porque o vLog tem keys | `REFUSE` | ficha | R005, R018 | R005 §3.4.2; R018 p. 1010 write cache “degrading reliability” — Fig. 2 desliga |
| L6 | MemTable skiplist/arena | `REFUSE` | rfc | R097 | keep `BTreeMap` até write path CPU-bound |
| L7 | Coluna no mesmo MANIFEST | `REFUSE` | rfc | R032, R066 | HTAP note §0 |
| L8 | Fold = LocalApplied, cursor após apply; **não** é voter | `SHIP` | ficha | R048; R096/R090 (listed) | RFC-0024; ficha R048 p. 2 — learner ≠ quorum |
| L9 | Pedra é local; produtos são camadas | `SHIP` | ficha | R010, R043, R044, R045 | RFC-0010; R010/R043/R044 layers; R045 p. 2–3 *é* o produto SQL-no-binário que **não** copiamos |
| L10 | Hardware PIM/CSD/FPGA como dep | `REFUSE` | listed | R071–R073 | P0 software |
| L22 | Protocolo Percolator (locks em colunas + 2PC cliente) | `REFUSE` | ficha | R041 | kernel = OCC+WAL; store = intents FDB; Fig. 8 = 4× no write pontual |
| L23 | Timestamp oracle como serviço | `REFUSE` | ficha | R041 | sequence LSM / `snapshot_begin`; 2 RPC/TX (p. 6) no caminho sub-ms |
| L24 | Read-index / SI no fold (estilo TiFlash §4.2.4) | `REFUSE` | ficha | R048 | RFC-0024: request path nunca `get_strong`; freshness do paper ≈ 1 s (Table 4) |
| L25 | WAL skip quando o host já tem log Raft/Paxos | `MEASURE` | ficha | R010 | p. 7; **≠ L5b**. Default Pedra continua sync. Gate: DST crash entre Raft commit e apply |
| L26 | Checksum de ficheiro SST no copy/checkpoint | `MEASURE` | ficha | R010 | p. 9–10; 17 mismatches/PB no wire; Pedra hoje é CRC de bloco |
| L27 | User timestamps first-class (fora da key) | `MEASURE` | ficha | R010 | Table 6 1.2–2×; ≠ L23 oracle. Só se um produto nomear snapshot cross-shard |
| L28 | Swarm / buggify no DST do store (fidelidade FDB §4) | `SHIP` | ficha | R043 | p. 8–9; já há `cluster_dst_*`. Programa: [RFC-0050](../docs/rfc/0050-world-in-tree-fdb-determinism.md) P2.2 + [RFC-0059](../docs/rfc/0059-massive-scale-parallel-dst-and-cluster-invariants.md) **P0 entregue** (2026-08-24): swarm paralelo determinístico (work-stealing, gate serial-vs-paralelo por `trace_hash`), backend mem com as mesmas seams, invariant checker cross-node no convergido, escala 7/9 nós. **Gate atingido** — bugs de cluster reais reproduzidos por seed in-tree e corrigidos com regressão pinada: seed 13 (frame sem CRC), seed 865 (índice reusado pós-escape), seed 49 (CommitUnknown por watermark), seed 104853 (InstallSnapshot stale-wipe). Campanhas 16384@3n / 4096@7n / 4096@9n-4ranges = 0 falhas ([evidência](../findings/2026-08-24-world-swarm/)) |
| L44 | π (PCT) × disk no `ConcurrentDb` / OCC — complemento G1/G5 do Sim2, não profundidade CPU-hours | `SHIP` | rfc | R043 | Apple client-testing 7.3.79: sim “not suitable” para multi-thread. Paper §4: não testa código fora de Flow. Programa: [RFC-0051](../docs/rfc/0051-beyond-fdb-sim-holes.md) **entregue P0+P1+P2** (2026-08-23): plantado d=2 seq/grosso CLEAN + PCT acha 3/256 + replay 8×; fence de grupo no fsync off-lock 9/256 (seq 0/256); OCC plantado 38/256 cross-group, correto 0/256; trial `FailingEnvArc<IoUringEnv>` Linux (skip explícito fora). **Não** claim “mais robustos que FDB” |
| L45 | DST recovery sob Miri / ASan FFI / mesmo seed TCG=`trace_hash` — caixas empilhadas no *ciclo*, não num interpretador | `MEASURE` | rfc | R043 | Paper §4: sim não testa FS/OS nem código fora de Flow. Programa: [RFC-0052](../docs/rfc/0052-dst-inside-boxes.md) (regras de empilhamento; slices executados pelo [RFC-0057](../docs/rfc/0057-maximum-intensity-parallel-dst-boxes-formal.md) P1). **P1 entregue (2026-08-24):** caixas no ciclo como jobs required do CI — `miri-dst-smoke` (supply-chain; 2 testes DST do pedradb-sim sob Miri, `MIRI_REQUIRED=1`), `tsan-box` (`race_job.sh` com `TSAN_REQUIRED=1`), `capi-asan-harness`; irmãos, nunca combinados no mesmo processo. Alvo `pct_concurrent` entregue: `pct_runner_without_pct_replays_and_covers_engine` (runner sobre `ConcurrentDb` real, políticas Sequential/RoundRobin — π PCT fora do processo, XOR 0052; TSan não roda em Apple Silicon, job Ubuntu é a autoridade). **REFUSE:** Miri-in-TCG, TCG-bench (finding 2026-08-22, distorção 11×), ASan+Miri no mesmo processo |
| L46 | Sanduíche IronFleet em Rust de produção (anos): TCB + crash-dictionary + 2ª máquina; **não** Dafny/`db.rs` ∀ | `SHIP` | rfc | — | IronFleet SOSP’15 §7 ~3.7 py; VeriBetrKV crash=IOSystem, 8× Rocks **recusado** como trade. Programa: [RFC-0053](../docs/rfc/0053-ironfleet-years.md) (Y1–Y3 entregues 2026-08-23) + face produto [RFC-0058](../docs/rfc/0058-verified-mode-kernel-derived-fallback.md) **P0+P1 entregues** (2026-08-24): `profile_report()` = os 44 kernels do catalog como composição declarada (machine-checked), pin lone-only, suíte FailingEnv/World/PCT no perfil (silent_wrong=0; fence ≤ 1 escritor), `open_verified` fold/lease/dcs/compat, derivação verified=full por oráculo, CI `verified-mode`, piso medido no modo (reads ≥5× vs Rocks default; writes lone-fsync ~0,001× — paridade **não** é claim do perfil); extração total segue REFUSE. Gate Y1: P40 iff no extract Aeneas + spec crash com dentes DST. **Não** claim “Pedra verificado” |
| L47 | Y1–Y3 shipped: caller refinements (vote/AE/apply), reopen kernel + lemmas recover→outcome, liveness bounded sob axioma quórum-vivo, handlers fail-closed; 169 ok/0 fail | `SHIP` | code | — | RFC-0053 **done**. π/VerusSync não disparado (RFC-0051 draft — estado gravado). Rácio prova:kernel ≈1:1 vs IronRSL 3.6:1 |
| L48 | Entregar o 100% relativo ao TCB = itens 1–11 do one-hundred-percent verdes, 12 contínuo, TCB congelado no CI; glue ~34k LOC → zero é o buraco dominante | `SHIP` | code | — | [RFC-0056](../docs/rfc/0056-one-hundred-percent-delivery.md) **done**: itens 1–6/8/9/11 verdes, 7 gated (RFC-0051 PCT), 10 “TCB à vista”, 12 contínuo. Estado: [one-hundred-percent-report.md](../docs/formal/one-hundred-percent-report.md) — 35 kernels/44 pares, liveness sob ES-1/2/3 (refutada sem axiomas), StdEnv sem ilhas, freeze no CI com 2 negativos |
| L29 | Clonar TS unbundled (Sequencer/Proxy/Resolver/LogServer) | `REFUSE` | ficha | R043, R045 | RFC-0017; R045 p. 3 é a geometria Multi-Raft 2f+1 que **fica** |
| L30 | Janela MVCC 5 s / TX 10 MB como default do kernel | `REFUSE` | ficha | R043 | p. 3, 11. Produto Montanha pode ter limite; Pedra sequence não |
| L31 | Índices = projecções KV na **mesma TX** (nunca Solr-eventual) | `SHIP` | ficha | R044 | p. 6, 9, Table 1; já `pedradb-index` + `RecordTable` seed |
| L32 | Record Layer produto (protobuf, record store, Cascades, TEXT/RANK) no kernel | `REFUSE` | ficha | R044 | p. 1–2, 11–12; `pedradb-sql` é prefixo; recipes ≠ Java RL |
| L33 | VERSION index (commit version ordenado) no fold/CHANGELOG | `MEASURE` | ficha | R044 | §7–8.1 p. 8–9. Gate: produto nomeia “scan since V” >2× mais barato que log-scan |
| L34 | Atomic-mutation indexes (SUM/COUNT via atómicos sem read conflict) no kernel | `REFUSE` | ficha | R044 | p. 3, 7; OCC Pedra serializaria o agregador. Layer pode RMW se o conflito for baixo |
| L35 | File-granular compact + least-overlap parent (LO+1) | `MEASURE` | ficha | R012 | p. 7 O2/TA I: Full 63×; partial −34–56%; LO+1 −10–23% vs outros partial. Pedra hoje = Full do par. Gate: WA/stall em `benches/baseline` com \(L=3\). **Antes** de L11 Spooky |
| L36 | Menu de 10 compaction policies / auto-switch | `REFUSE` | ficha | R012 | p. 2 A; p. 12 visão sem número. Uma policy que o DST explica |
| L37 | Universal / tiering como compact default | `REFUSE` | ficha | R012, R014 | R012 TA II; R014 Table 3 — robusto **sempre** Leveling |
| L38 | FADE: TTL por nível + pick por a_max/b (TSD/TSA) | `MEASURE` | ficha | R031, R012 | R031 p. 898–900, Fig. 6. **Depois de L35** (precisa file pick). Gate: produto nomeia Dth (7/30/60 d). Sem SLA, Full-do-par L=3 pode já chegar — medir a_max. R012 +18–35% é *outro* bench. **Não** `compact_for_reads` como política de delete |
| L43 | KiWi / delete tiles / layout S×D | `REFUSE` | ficha | R031 | p. 900–905. Pedra sem delete key secundária; Bloom-por-SST; h>1 multiplica I/O de point (Table 2). Correlação S–D≈1 (seq) → paper diz h=1. Layer se um produto tiver timestamp≠key *e* range ≥1/30 |
| L39 | Multi-Raft: split/merge de Ranges + leaseholder | `MEASURE` | ficha | R045 | p. 2–3, 64 MiB, lease 4.5 s, Joint Consensus p. 12. Gate: store com **>1** grupo Raft. `pedradb-raft` hoje é 1 grupo |
| L40 | Protocolo TX CRDB (HLC 500 ms + uncertainty + intents+staging) no kernel | `REFUSE` | ficha | R045 | p. 5–8. Kernel = OCC+sequence (L30). Store 2PC FDB fica. Parallel Commits no store só no gate RFC-0017 P1 |
| L19 | Hash-partitioned value store (grupos + GC sem get LSM) | `MEASURE` | ficha | R018 | p. 1010–1014; RFC-0028. Gate: soak values grandes em que rewrite/blobs WA ≥ Rocks Fig. 2 *e* key no record. **Não** SHIP (0026-C). RAID/cache fora |
| L11 | Compact granulado Spooky (\(X=L-2\), fronteiras de \(L\)) | `MEASURE` | ficha | R013 | p. 1, 10–12. Gate: **L35 shipped** + utilização SSD nomeada (`nvme` GC) + \(L\ge 4\) ou \(L\) ≫ par. Pedra hoje = Full-do-par, \(L\le 3\) |
| L12 | Endure: default *estático* robusto (T, bits, policy) na bola KL | `MEASURE` | ficha | R014 | p. 2–4, 12 Table 3. Gate: ≥2 knobs {T, Monkey bits, L\|T}. Hoje 0. **Não** tuner online (p. 2; L36). Robusto sempre Leveling — confirma L3/L37 |
| L41 | Stall: nomes MT/L0/PS (= MMO/L0O/RDO) + prioridade flush > L0-compact > L≥1 | `MEASURE` | ficha | R017, R016 | R017 p. 756–758; R016 p. 67, 70, Table 2. Gate: soak p99 write ≥10× p50 *e* causa Cm/L0 (não WAL sync). Limiter só `Env`-off no DST. **Não** SILK completo; **não** tuner ADOC (L42) |
| L42 | ADOC tuner live (AIMD threads+batch, Tw=1 s) | `REFUSE` | ficha | R016 | p. 72–76. Pedra: 0 knobs vivos (flush single-flight, 4 MiB, sem L0-stop/PS). DST-hostil. Complementar a L41 (taxonomia). Reabrir só se L41 soak nomear overflow *e* existir superfície ≥2 knobs (eco L12) |

## B. Aberto pelo catálogo (ainda `listed` — não é prova)

| ID | Decisão candidata | Tag | src | Papers | Fecha se |
|----|-------------------|-----|-----|--------|----------|
| L13 | Disco / REMIX range index | `MEASURE` | listed | R022, R023 | `scan` for um problema |
| L14 | Splinter/Turtle em vez de LSM | `MEASURE` | listed | R029, R040 | perder workload nomeado por &gt;2× |
| L15 | Learned index no SST | `REFUSE` | listed | R019, R098 | só se Bourbon ficha D3 disser o contrário |
| L16 | Compact-as-a-service | `REFUSE` | listed | R059 | local compact chato e correto |
| L17 | Value-store menu → **C** (blobs) | `SHIP` | bench | R005, R018 | [RFC-0026](../docs/rfc/0026-value-store-evolution-menu.md) P0.3; [0029 P0](../docs/rfc/0029-blob-generations-and-scan-prefetch.md) landed |
| L18 | Incremental tail GC (WiscKey) | `MEASURE` | ficha | R005, R018 | [RFC-0027](../docs/rfc/0027-incremental-vlog-gc.md) — ficha R018: tail é o desenho da Fig. 2 (19.7×). Não SHIP como próximo GC |
| L20 | Blob files + discardable-ratio + prefetch | `SHIP` | ficha | R005 | [RFC-0029](../docs/rfc/0029-blob-generations-and-scan-prefetch.md) P0–P2 + auto θ + write-path auto GC (incl. ConcurrentDb) |
| L21 | `read_ptr_on` file 0 ≠ active blob path | `SHIP` | code | — | VLG1 after rotate; test `read_ptr_on_file0_after_blob_handle` |

## C. Falsos / bloqueados

_Nenhum ainda. Uma linha `FALSE` exige ficha que mostre o mismatch
(hardware, Rocks version, workload)._

## Como acrescentar

1. Ficha ≥ D3 (ou `src=listed` consciente, no bloco B).
2. Uma linha aqui, mesmo id estável (`Lnn`).
3. Se contradisser RFC-0012/0014, editar o RFC **na mesma mudança**.
