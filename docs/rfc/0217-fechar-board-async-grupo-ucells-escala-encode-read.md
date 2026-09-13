# RFC: 0217 — Fechar o board async: grupo em baixa concorrência, U-cells nativas, escala pesada, encode off-path, read-side e produto

**Status:** in-progress
**Updated:** 2026-09-13

## Background

O board async same-class (Pedra `PEDRA_PARITY_ASYNC=1` vs RocksDB default
`sync=false`, peer `ROCKS_PARITY_SYNC=0`, mesmo round/boot) tem hoje:

- **15/15 formas smoke ≥ 1,254** (gate oficial, `rocks-parity-floor1x/`) e
  20/20 no sweep p201o single-client — single-client e leitura pagos.
- **Fronteira de concorrência**: writers 2–4 perdem (`ycsb_f_mc4` cartaz
  Linux **0,491** default / **0,836** braço rmw p211m; DIAG Darwin mc2/mc3
  0,25–0,39), writers ≥ 6 o drenar-grupo amortiza (DIAG mc6/mc8 rmw
  1,97×/2,23× sobre o braço clean), mc50 paga (`kvrocks_set_mc50` 1,678×).
  Mecanismo medido (PHASE/wg p211p/p211q): `avg_grp` 1,00 em TODAS as
  contagens no default (nunca forma grupo); rmw 1,04→1,14→2,1→3,1
  (mc2/3/6/8) com `lwait` 0,2–1,8µs. Entre mc8 e mc50 não há ponto medido.
- **6 perdas U-cells nomeadas** (Linux 3-run, `p209-ucells-gate/`):
  kafka_changelog_flush **0,036**; ingest_sst **0,069**;
  compaction_filter_drop **0,081**; linkbench_mix **0,236**; wbwi **0,494**;
  myrocks_write_tx **0,751**.
- **Escala pesada sem meter**: prefix 100M @4GiB **0,70×**; overwrite_mc4
  25M @4GiB **0,557×** (3/3); 15M sem número — mecanismos 0193/0194/0195
  aterrizados, meter blocked (orçamento de onda + custo, datado).
- **Encode de memtable por commit**: `mem=3,13–14,2µs` (2º dono
  quantificado; −0,180 do déficit de kvrocks_set_mc50 é mem amortizado).
- **Read-side −18%** (F8): `deps_scan` single 0,831 (p201o).
- **Escada de admissão** (produto): sob banda Reclaim (FS pequeno, free <
  `DISK_SOFT_FREE_BYTES`=256MiB) o admit custa dezenas de ms por commit
  (14–18ms macOS/F_PREALLOCATE; 6–7ms Linux tmpfs) + parks 230–310ms/op
  (20–60× no tempo de célula).
- **Linha G1 single-client write-per-op** (0,001–0,056×): fd-ceiling por
  construção (p50 F_FULLFSYNC 3,8ms ⇒ ~1/p50; Linux fdatasync p50 25,7µs ⇒
  0,056×) — prova viva na família
  `rfc0041_one_fdatasync_cannot_hit_2x_rocks_default_ycsb_a`.

**Errata que re-ancora este RFC (commit `426272f4`, 2026-09-13):** o
veredito P1.2 nomeou o dono do residual 0,836 como "fdatasync per-commit da
coluna de paridade" — **falso no código**: `PEDRA_PARITY_ASYNC=1` faz
`opts.set_sync(false)` (`rocksdb-parity-bench/src/engines.rs:83-85`) e o
commit async só sincroniza sob plano G1 (`WalCommitPlan::AppendSync*`;
`concurrent.rs:1228-1243`: "Async: write() per group, no fdatasync").
Telemetria Darwin (`flsh≈0,03µs`, `wal≈1µs`) com células mc2–mc8 SUB-1
(0,25–0,84 DIAG) prova que o dono existe **sem barreira nenhuma**: é a
**seção serial por commit** — write() do WAL sob mutex + encode de memtable
+ publish sob write-lock — contra o memcpy userspace por writer do Rocks.
O fd-ceiling é mecanismo exclusivo da G1. A absolvição do escalonador
(P1.2, dois extremos) permanece de pé.

Pain/why now: RFC-0211 fechou sem caixa aberta e adjudicou o mecanismo
drenar-grupo; a direção do usuário é **resolver todos os pontos fracos do
board**, estendendo este RFC a cada etapa nova descoberta.

## Problems This Solves

- **Problem:** writers 2–4 pagam a seção serial inteira por commit (ratio
  0,49–0,84) porque o default nunca forma grupo (`async_merge_policy` só
  mergeia quando writers > ncpu; `avg_grp=1,00` sempre).
- **Problem:** seis formas de workload real (flush-por-op, ingestão SST,
  filtro de compaction, mix delete/scan, batch indexado, tx N-updates)
  perdem por rodar em emulação do compat, não em caminho nativo.
- **Problem:** as duas células de 4 GiB (0,70/0,557) estão sem meter desde
  que os mecanismos aterrizzaram — nem claim, nem fechamento.
- **Problem:** encode de memtable (3,13–14,2µs) mora no caminho serial do
  líder/membro — quantificado como 2º dono.
- **Problem:** scan/cursor −18% no lado de leitura.
- **Problem:** sob pressão de disco o admit por commit custa dezenas de ms
  (produto: qualquer deploy em FS pequeno).
- **Problem:** eixos inteiros sem número (delete-heavy, mc9–49, 1 GiB,
  p99) — cobertura para poder publicar o board com perdas nomeadas.

## Proposed Solution

- **P0:** formar grupo em baixa concorrência — janela de coleta bounded no
  líder (`PEDRA_GROUP_WINDOW_US`, default off) + attach de chegadas ao
  grupo in-flight; merge elegível a partir de 2 writers quando a janela
  está on. Amortiza write()+encode (async) e a barreira real (G1 mc≥2).
- **P1:** tirar as 6 U-cells da emulação — caminhos nativos (flush
  amortizado, SST write direto + hook de filtro no compactor, batch
  indexado nativo, tx single-writer real) ou veredito datado de teto com
  número.
- **P2:** fechar escala pesada (meter 4 GiB), encode off-path, cursor de
  scan, histerese do admit (produto) e os eixos de cobertura sem número.
- **Adjudicação formal (non-goal):** G1 single-client write-per-op é
  teto por construção — documentado, nunca win, nunca escondido.

## Delivery slices (mandatory)

### P0 — Grupo em baixa concorrência (ranking nº 1; residual 0,491/0,836)

- [x] **P0.1** Kernel da janela de coleta: `group_window_kernel` puro
  (decisão bounded: espera W µs no líder quando ≥2 writers ativos e merge
  elegível; zeros para lone/single; twin AS-IS) + wiring no caminho real
  (`submit_after_begin`/WriteGroup: `PEDRA_GROUP_WINDOW_US=N` torna o
  merge elegível em writers ≥2 e o líder espera W antes de drenar;
  caminho lone `active==1 ∧ ¬recently_concurrent` intocado) + testes
  `rfc0217_group_window_*` no caminho real. **P0.1b** (smokes
  2026-09-13, Darwin DIAG): a janela flat sozinha deixou `avg_grp`
  1,16 no ycsb_f mc2 (o par no gap client-side é invisível a `active` —
  o contador cai no consumo da reply); três braços fecharam o fantasma:
  (1) coleta atravessando o gap no líder (wait com saída por quiessência
  de 20 µs após o primeiro absorb, bound = janela), (2) horizonte de par
  estendido à janela (`peer_horizon_us`; bypass lone não rouba o líder),
  (3) merge elegível por par recente (`merge_eligible(w, win, peers)`;
  sem isso o bypass do write-lock comita solo sem líder exatamente no
  regime-alvo). Resultado: ycsb_f mc2 `avg_grp` 1,00 → **1,92** (96% do
  teto físico 2,0; cache_overwrite mc2 1,88). — status: `done`
- [x] **P0.2** Attach in-flight: chegada durante o dreno/write do líder
  entra no mesmo voo (fold/stage na janela off-lock; na G1, attach também
  durante a barreira do grupo) + testes `rfc0217_inflight_attach_*`.
  **Adjudicado 2026-09-13** (dados P0.1b/P0.3): com a coleta pelo gap,
  `avg_grp` já atinge ~96% do teto (voos cheios — o perdedor do publish
  entra no próximo voo, também cheio, sem estacionar extra), então o
  attach no mesmo voo não move `avg_grp` nem throughput; o ganho residual
  é 1 ciclo de latência por membro. Attach verdadeiro exige encode
  member-side + seq sem o write guard (estruturalmente P2.2) — **fatia
  fundida em P2.2**, reabre se o ratio quiet do P0.3 ficar <0,9 com voos
  cheios. — status: `done`
- [ ] **P0.3** Meter DIAG Darwin (driver host p211p/p211q): frontier
  mc2/3/4/6/8, braços window on/off, PHASE/wg; alvo `avg_grp` mc2–mc4
  ≥2,0 e ratio DIAG mc2/mc3 saindo de 0,25–0,39 → ≥0,9; guardas
  deps_apply_batch/mc50. **Parcial 2026-09-13** (commit `234001f7`,
  driver 3 rounds `p0217-host-driver.sh`): avg_grp window mc2
  1,91–1,94 (teto físico 2,0; ≥96%), mc3 2,79–2,93, mc4 3,35–3,65,
  mc50 10,26 → 24,38; lwait bypass mc8 21,7µs → 0,1µs; guardas
  apply_batch window/clean 0,92–1,54× (≥1 exceto mc2 0,92);
  **ratios inutilizáveis** — hostload 16–39 (node externos + caixote-api;
  clean mc8 min 0,19 com round 7,35× = ruído puro). Alvo de ratio
  aguarda caixa quieta (re-run pendente). — status: `doing`
- [ ] **P0.4** Meter Linux gate 3-run quiet min-of-3 (âncora p149):
  `ycsb_f_mc4` default 0,491 → **≥1,0** com janela on; guardas ≥ nível
  p211m (ycsb_a_mc4, overwrite_mc4, apply_mc4, mc50); cartazes pagos em
  guarda ≥; veredito datado; **flip do default só com este meter
  válido**; se o gate seguir blocked, veredito gate-blocked datado.
  **Gate-blocked 2026-09-13T04:42Z**: único host BYOC conectado é o
  MacBook (aarch64); p149 desconectado; deploy `0caaac0b` pending. Binário
  amd64 com P0.1b + driver `p0217-linux-driver.sh` prontos para disparar
  no retorno do host. — status: `doing`
- [ ] **P0.5** Re-adjudicação do dono no Linux (errata `426272f4`): onda
  admission-clean com PHASE distribuindo a seção serial no âncora ext4
  (wal/mem/publish/lwait por commit); finding datado. **Mesmo block do
  P0.4** (mesma onda, mesmo host; 2026-09-13T04:42Z). — status: `doing`

### P1 — U-cells nativas (ranking nº 2; cada fatia: ≥1,0 OU teto datado com número)

- [x] **P1.1** kafka_changelog_flush 0,036: flush amortizado no compat
  (pipeline completo por chamada — snapshot do hang: 1331/1793 amostras em
  `compact_gate` — vira batch/staged com gate) sem mudar a semântica de
  durabilidade do flush explícito. **Feito 2026-09-13** (mecanismo +
  testes; números abaixo são DIAG Darwin, nunca claim — ratio ≥1,0 é o
  meter Linux, fatia e4b): kernel `changelog_flush_store_now` +
  `wal_rotate_archives` (debounce 64 flushes, cap 64 archives) — o rotate
  decide por cobertura (`walless_covered`, `unpublished_below_floor`,
  feed settled) e publica durável só quando `!settled ∧ (¬archive_now ∨
  store_now)`; a janela WAL-less acima do floor PUBLICA (nunca arquiva —
  frames ≤ floor já cobertos pelo manifesto); frames não-publicados >
  floor arquivam e o replay no open é filtrado por
  `sequence > manifest_floor`. Três hazards de durabilidade fechados com
  guards: (1) `lookup` resolve mem-vs-SST sem comparar seq → replay
  incondicional sombreava publicados; (2) seqs de bulk intercalam com
  puts → o floor não prova cobertura ≤ floor; (3) tail meta 1-key e bulk
  runs são WAL-less → `walless_seq_high` trava truncate. Drain de
  archives com budget (`WAL_ARCHIVE_UNLINK_BUDGET=4`/chamada; cap conta
  `wal_archive_live()`, senão o gate dispararia store a cada rotate
  durante o drain). Naming dos archives (`WAL.archNNNN`,
  `wal_archive_slot_name`/`wal_archive_slot_of`) ficou em db.rs, não no
  kernel: `str::pattern`/`format!` é intraduzível na lane aeneas/charon
  (extração re-carimbada verde, sem arquivo parcial).
  `verify_checksums` vira subset-check (manifesto ⊆
  memória — F196 com publish adiado; manifesto nomeando arquivo ausente
  continua `CorruptManifest`). Suite `rfc0217_changelog_flush_amortized`
  6/6 (defer/crash-reopen, debounce+drain, cap→store síncrono,
  crash-feed-parity, below-floor-publishes, publish-at-gate); 4 testes
  as-is recontratados para o flush adiado (`crash_after_flush…`,
  `idle_rotate…`, 2 de `verify_checksums`); A/B lib 936 pass / 23 fail =
  baseline (zero novas). DIAG Darwin OPS=300 batch=32: p50 9,2ms pré →
  **7,21ms** (5,93 pré-budget), p99 290ms → **72ms** com o budget de
  unlinks, stores 15/1000 (1/64, geração de MANIFEST); rocks twin
  0,386ms. — status: `done`
- [x] **P1.2** ingest_sst 0,069 + compaction_filter_drop 0,081: sair da
  emulação — ingest = escrita SST direta + install; filter = hook real no
  compactor. — status: `done` `92a76a97` — `Db::ingest_sst_file` (seq
  globais frescas, rewrite+install L0, MANIFEST durável antes do Ok,
  flush-first parity `allow_write_flush`) + `CompactFilterDecision`/
  `FilterMergeSource` no merge (Remove = 1 tombstone no topo da run —
  sem ressurreição no replay; invalidação wholesale dos caches de leitura
  — regressão de cache-stale pega no A/B); compat nativo
  (`compact`/`compact_with_filter`/ingest default-CF),
  `apply_compaction_filter` removido. Suite `rfc0217_native_ingest_filter`
  8/8; core `--tests` 937/23/4 = baseline (0 novas); compat 94/3 (3
  pré-existentes em HEAD `52afb58c`). DIAG Darwin n=300 (não-claim):
  filter 434→835 qps (0,47→0,935 do twin rocks 893; p99 21,3→7,8ms);
  ingest p50 0,83→0,60ms. Cartaz Linux 3-run = meter e4b (p149).
- [ ] **P1.3** wbwi 0,494 + myrocks_write_tx 0,751: batch indexado nativo
  + tx single-writer real (`begin_occ`/commit já existem) no lugar do
  WriteBatch emulado. **DIAG micro 2026-09-13** (Darwin, rocksapi,
  n=300, 3 rounds quiet, peer sync=0): flat overlay `5c1f5b43` move
  0,307 → 0,317/0,324/0,349 (min +3%, best +13% relativo) — ganho real
  mas perda honesta permanece; dono do residual = emulação WBWI
  (per-op ~369ns vs ~117ns do WriteBatchWithIndex nativo; write_tx
  nativo e cartaz Linux 0,494 = meter e4b). — status: `doing`
- [ ] **P1.4** linkbench_mix 0,236: decompor primeiro (mix scan+delete;
  telemetria read_probe/scan), atacar o dono nomeado. — status: `doing`
  (ataque landed `2f083efb`; veredito ratio = e4b).
  Decomposição (DIAG Darwin n=3000, sample macOS + probe): write-side =
  `fcntl(F_FULLFSYNC)` 92,9% do wall (Darwin-only: std `sync_data` em
  apple = F_FULLFSYNC 4,18ms/op vs `fsync()` 0,053ms que o rocks usa —
  80× estrutural, não existe no Linux; fsync_test.c no scratch). Reads
  do mix pagavam re-sort do `point_ord`: com myrocks/deps o engine abre
  multi-CF → keys `default\0…` caem no shard point (HashMap) e cada um
  dos ~750 write→scan rebuildava collect+sort de ~104k entradas
  (~120µs). Ataque: `point_ord_btree` incremental (build 1x no primeiro
  range count, insert O(log n) por put quando a view existe, skip de 1
  atomic quando não — shapes sem scan pagam zero; vale também para o
  point-path do `last_visible_under_prefix`). DIAG pós: p50 17,7→8,5µs,
  scan 1,1µs/op, `ord_builds=0` na janela do mix (o build único foi pago
  no `read_only` anterior). A/B: core --tests 937/23/4 = baseline;
  compat 95/4 (os 4 r0218 pré-existentes). Sobra Linux (e4b): serial
  write 4,5µs/commit + read 1,1µs/op vs rocks p50 6,4µs/read.

### P2 — Escala pesada, encode, read-side, produto, cobertura

- [ ] **P2.1** Escala pesada: meter e fechar prefix 100M @4GiB
  (0,70→≥1,0), overwrite_mc4 25M (0,557→≥1,0) e 15M (sem número→≥1,0);
  blocked atual: orçamento de onda + custo, datado. — status: `todo`
- [ ] **P2.2** Encode de memtable off-path: encode no membro antes do
  grupo ou batch-encode no apply (alvo: `mem=` saindo de 3,13–14,2µs do
  caminho do líder; recuperar o −0,180 do déficit kvrocks_set_mc50). —
  status: `doing` (batch-encode no apply landed `9b5ca0f5`; DIAG de fase
  = fila p22-diag). Landed: `insert_many` (o apply do grupo) agora usa
  memo batch-local de prefixo — slot `&mut` do shard `tail_idx` mantido
  através do loop (mata 1 `Bytes::copy_from_slice` + 1 walk por op),
  acumulador de delta por CF com 1 flush (mata 1 walk por op em
  `cf_bytes`), slot do `cf_span` (mata 1 walk + re-check por op);
  `shard_insert` extraído e compartilhado com `tail_append` (paths 1-op
  inalterados — controle: lone `kvrocks_set` não deve mover). Equivalência
  provada por teste novo (troca de prefixo point/short/long/one-slash,
  replace same-seq <16, tombstone, range-del, re-insert em shard
  existente, cf_bytes/cf_span comparados); o teste pegou 1 divergência
  real no span durante o desenvolvimento (entrada recém-criada
  re-checada) e ela foi corrigida. A/B: core `--lib` 937/23 = baseline
  idêntico (1 flaky `rfc0167_l0_stall` falhou só no baseline); bench
  `compat_vs_rocks` 6/6; lib bench 28/2 = falhas pré-existentes idem no
  worktree baseline. Faltam: DIAG `mem=` A/B (base vs patch, intercalado,
  gate quiet) e o cartaz Linux (e4b). Encode member-side (pré-grupo no
  cliente) fica como follow-up se o DIAG mostrar `mem=` ainda ≥1µs/op.
- [ ] **P2.3** Read-side −18%: decompor cursor do scan (`deps_scan`
  single 0,831 DIAG p201o → ≥1,0 no cartaz Linux). — status: `todo`
- [ ] **P2.4** Escada de admissão (produto): probe com histerese/cache
  curto em vez de por commit (alvo ≤1ms/commit sob Reclaim; parks
  230–310ms eliminados do caminho quente; guarda: semântica Refuse abaixo
  do hard intacta, `disk_pressure` 11/11). — status: `doing` (kernel +
  wiring landed, knob `PEDRA_DISK_PROBE_CACHE_MS` opt-in, default off até
  meter). Landed: `probe_cached`/`reclaim_ladder_due` no
  `disk_pressure_kernel` (puras, testadas) + wiring em
  `ensure_disk_pressure_admitted` — (a) veredito `Ok` fresco (< janela,
  default 200ms) reusado sem `statvfs`; (b) banda soft SÓ proeba por
  commit e a escada (compact/rotate/vlog-GC) rate-limited a 1×/1000ms —
  o commit admite sem esperar a escada (mata as dezenas de ms/commit);
  (c) Refuse nunca é mascarado fora da janela: banda soft e refuse
  nunca populam o cache (`disk_ok_probe_at=None`), e o teste prova com
  contador de probes que o Ok cacheado não re-proeba e que o refuse
  abaixo do hard volta no commit seguinte ao expirar a janela. Guardiãs:
  `disk_pressure` 13/13 (11 originais + 2 novas de kernel), 2 testes Db
  novos no SpaceEnv real (`put` end-to-end com probe contável).
  Pendente: meter Linux em disco pequeno (efeito ≤1ms/commit + parks) e
  decidir flip do default (off até lá).
- [ ] **P2.5** Cobertura dos eixos sem número: sweep delete-heavy (hat:
  perda — única família tocando deletes hoje é perda), concorrência 9–49
  (fronteira mc8→mc50 sem ponto), célula 1 GiB (faixa smoke↔4GiB vazia),
  captura p99/p999 (todas as comparações atuais são throughput). —
  status: `todo`

### Regra de extensão (board aberto)

Este RFC **permanece aberto** até toda célula nomeada do board estar ≥1,0
ou carregar veredito datado de teto-por-construção. Qualquer onda deste
RFC que descubra célula nova <1,0 abre fatia datada nova aqui (P2.6,
P2.7, …) no mesmo commit do finding — nada é descoberto e deixado sem
dono.

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Kernel janela de coleta + wiring real + testes `rfc0217_group_window_*` | done | `010f61fe` + P0.1b `234001f7` | 2026-09-13 |
| P0.2 | p0 | Attach in-flight: adjudicado — fundido em P2.2 (encode member-side; voos já cheios) | done | `234001f7` | 2026-09-13 |
| P0.3 | p0 | Meter DIAG Darwin: avg_grp ok (mc2 1,93/mc3 2,9/mc4 3,5/mc50 24,4); ratio espera caixa quieta | doing | `234001f7` | 2026-09-13 |
| P0.4 | p0 | Meter Linux 3-run quiet: gate-blocked 04:42Z (p149 desconectado); binário+driver prontos | doing | — | 2026-09-13 |
| P0.5 | p0 | Re-adjudicação do dono no Linux: mesmo block do P0.4 | doing | — | 2026-09-13 |
| P1.1 | p1 | kafka_changelog_flush: flush amortizado | done | `ded231ab` (ratio ≥1,0 = meter Linux e4b) | 2026-09-13 |
| P1.2 | p1 | ingest_sst + compaction_filter: caminhos nativos | done | `92a76a97` (cartaz Linux = e4b; DIAG filter 0,47→0,935) | 2026-09-13 |
| P1.3 | p1 | wbwi + write_tx: batch indexado + tx nativos | doing | `5c1f5b43` + DIAG micro 09-13: 0,307→0,317–0,349 (perda honesta; cartaz = e4b) | 2026-09-13 |
| P1.4 | p1 | linkbench_mix: decompor + atacar dono | doing | `2f083efb` (point_ord_btree incremental; DIAG p50 −52%; cartaz = e4b) | 2026-09-13 |
| P2.1 | p2 | Escala pesada 4GiB: meter + fechar (0,70/0,557) | todo | — | 2026-09-13 |
| P2.2 | p2 | Encode memtable off-path | doing | `9b5ca0f5` (memo batch-local no apply; DIAG fase + cartaz = e4b) | 2026-09-13 |
| P2.3 | p2 | Read-side: cursor de scan | todo | — | 2026-09-13 |
| P2.4 | p2 | Escada de admissão: histerese (produto) | doing | `c4fe195d` (knob opt-in; meter disco pequeno p/ flip default = e4b) | 2026-09-13 |
| P2.5 | p2 | Cobertura: delete-heavy, mc9–49, 1GiB, p99 | todo | — | 2026-09-13 |

## Acceptance Criteria

- **Tests:** `rfc0217_group_window_*` e `rfc0217_inflight_attach_*` no
  caminho real (`ConcurrentDb` + env real, env pin on/off); guarda do
  caminho lone (single-client p50 sem janela — sem regressão); twin AS-IS
  com env off idêntico ao comportamento atual; `disk_pressure` 11/11
  intactas (P2.4); serial A/B ⊆ baseline conhecido + musl exit 0 a cada
  fatia de código.
- **Telemetry:** `write_group_stats` (`avg_grp` alvo ≥2 em mc2–mc4),
  `write_phase_stats` (decomposição da seção serial no Linux), contadores
  do flush amortizado (P1.1), read_probe/scan (P1.4), parks do admit
  (P2.4) — sempre na linha existente das ondas p211p/p211q.
- **Documentation:** finding datado por onda; linha viva no
  `docs/status.md`; inventário rev. 4 ao fechar o board; errata
  `426272f4` referenciada (dono corrigido).
- **Screenshots:** none — backend-only.

## Out of scope

- Ratios contra peer `sync=true` (nunca win; `rocks-parity-compare`
  exits 2) e qualquer re-medida de cartaz pago (apply_mc4 1,0859;
  kvrocks_set_mc50 1,678; G1 apply_mc4 2,788 head3) — guardas ≥ apenas.
- Mudar o contrato G1 (fdatasync antes de Ok é o produto); a linha G1
  single-client write-per-op fica no teto por construção, documentada,
  nunca citada como win nem escondida.
- Mass-fix das 23 falhas formais shared-lane (sessão separada); RFCs da
  sessão paralela (0216); multi-node/Montanha; Darwin como claim (DIAG).
