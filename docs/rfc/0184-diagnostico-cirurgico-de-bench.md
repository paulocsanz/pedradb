# RFC-0184 — Diagnóstico cirúrgico de um cell de bench

**Status:** in-progress
**Updated:** 2026-09-08
**ID:** 0184
**Parents:** [0176](0176-modelo-matematico-de-escala.md),
[0183](0183-teto-apply-serial-e-1c.md),
[0168](0168-vitoria-por-celula-toda-escala.md) (cost trace)
**Peer:** RocksDB default `sync=false`. G1 não é win.

> P0 é o kernel + `pedra diagnose`. Caixa 4 GiB / 3-run Linux é P1.
> Darwin DIAG não é cartaz Linux.

## Background

- RFC-0176 prevê **get**: \(T=P\cdot(H\tau_{\mathrm{ram}}+(1-H)\tau_{\mathrm{disk}})\cdot(1+\eta)\).
  CLI `pedra scale-model` existe no tree interno; o binário público
  `pedra` só corria `scale` (ladder 1M/25M/100M).
  P2.35 decompõe o mesmo get em `GetWork × MachineSpec` (bloom/index/block/`pread`
  × L1/L2/L3/DRAM/SSD). Envelope 0176 fica; o composicional é o lower
  bound da spec (não llvm-mca, não η).
- WRITEPHASE (`PEDRA_WRITE_PHASE_STATS=1`) soma prepare/wal/mem/publish/
  flush_check/lock_wait. O harness imprimia µs e o humano adivinhava
  o corte. 0183 teve de fazer a conta à mão: 1c = WAL; apply = flush_check
  148 µs/commit; mem/gap = 2,7% → 0055 parked.
- `PEDRA_COST_TRACE` conta SST probes (get/prefix). Não classifica contra
  \(P_{\mathrm{best}}\) vs walk-all.
- Darwin same-boot 0182 (0,58–0,67 misto) **não** é o Linux. Linux 1c
  oficial já é 2,4–3,5×; `ycsb_a_mc4` 25M **2,26×**; o buraco Linux
  nomeado é `overwrite_mc4` 25M **0,557×** (caixa não remeteu após 0180).

## Problems This Solves

- **Problem:** um ratio <1× não diz *qual* timer cortar.
- **Problem:** skiplist / concurrent memtable aparecem como palpite
  quando o número é WAL ou flush_check.
- **Problem:** get lento não distingue happy (disco×η) de as-is
  (walk \(N_{\mathrm{files}}\)).

## Proposed Solution

Kernel puro [`bench_gap_kernel`]: mesmos ns → mesmo `lever`. WRITEPHASE
rankeia o timer; mem/gap ≥15% **e** mc despark 0055; n=2–8 com
avg_group <1,2 → grouping; n≥16 lock convoy. Get: mede vs
best/happy/worst/as-is do 0176. CLI `pedra diagnose`. Harness imprime
a linha quando há phasesΔ. Sem harness novo.

## Delivery slices (mandatory)

### P0 — kernel + CLI (este host)

- [x] **P0.1** Este RFC — status: `done`
- [x] **P0.2** `bench_gap_kernel` + dentes 0183 (1c=WAL, apply=flush_check,
      1c não despark) + get as-is walk — status: `done`
- [x] **P0.3** `pedra diagnose write|get` + linha no
      `rocks-parity-bench` após phasesΔ — status: `done`
- [x] **P0.4** Mixed `get_path` (`read_pct≥40`) + `classify_probes` +
      `balance_admits` (DIAG recusa; named-loss Linux ou ≥2 cartaz).
      Set `BALANCE_SHAPES` (0182). — status: `done`
- [x] **P0.5** Async apply/`commit_async_ops` usa stage-only; CF
      **default-over** parka a mem inteira (O(1)), não
      `take_family("default")` O(n). Não parka só porque o global estourou
      (RFC-0159 P1.3: CF data > cap global). Named CF continua
      `take_family` contíguo. Teste
      `rfc0184_async_ops_does_not_write_l0_when_over_limit`.
      — status: `done`
- [x] **P0.6** `commit_async_ops` pina `commit_inflight` (mesmo contrato
      que 1-op RFC-0180 P0.25). Compact host não barganha o write lock
      entre batches apply/bypass. Pin dropa no Ok.
      — status: `done`
- [x] **P0.7** `apply_batch_with` / TX async Ok stage/park, não
      `flush_cf` (espelho P0.14/P0.5). G1 continua a poder escrever L0.
      Teste `rfc0184_async_apply_batch_does_not_write_l0_when_over_limit`.
      — status: `done`

### P1 — Linux

- [ ] **P1.1** `overwrite_mc4` isolado na caixa + diagnose (0178 P1.3) —
      status: `todo`
- [x] **P1.2** compare JSON inclui `diagnose.lever` — status: `done`

### P2 — polish

- [x] **P2.1** `deps_raftlog` / `deps_raftlog_mcN` WRITEPHASE →
      `diagnose.lever` no JSON (1c já tinha phasesΔ sem attach; mc não
      tinha snapshot). — status: `done`
- [x] **P2.2** YCSB 1c `run()` WRITEPHASE → `diagnose.lever` (A/F
      passam `read_pct`; mc já tinha). — status: `done`
- [x] **P2.3** `kvrocks_set_mc50` WRITEPHASE → `diagnose.lever`
      (`lock_convoy` n≥16; kernel `mc50_bypass_is_lock_convoy`).
      — status: `done`
- [x] **P2.4** scale `probe_miss` + `PEDRA_COST_TRACE` →
      `classify_probes` vs \(P_{\mathrm{best}}\) (walk-all ≠ "disk").
      — status: `done`
- [x] **P2.5** scale `prefix_scan` + COST_TRACE → `classify_probes`
      (`scan_sst_probed` / op vs \(P_{\mathrm{best}}\)). — status: `done`
- [x] **P2.6** scale `get_hit` measured ns → `classify_get` vs 0176
      clock (best/happy/worst/as_is). Sempre, não só COST_TRACE.
      — status: `done`
- [x] **P2.7** scale `lookup_100` / get_loop measured ns/100 →
      `classify_get` vs 0176. — status: `done`
- [x] **P2.8** Quicksilver `qs_hot_get` / `qs_neg_lookup` /
      `qs_batch_write` WRITEPHASE → `diagnose.lever` (hot/neg =
      `get_path`; batch = write lever). Kernel: `read_pct≥40` e
      timed=0 ⇒ `get_path` (não `prepare`). — status: `done`
- [x] **P2.9** kvrocks 1c (`get`/`set`/`pipelined_set`/`scan`/
      `blob_set`) WRITEPHASE → `diagnose.lever` (get/scan =
      `get_path`; set = write lever). mc50 já tinha (P2.3).
      — status: `done`
- [x] **P2.10** MyRocks `point_select` / `read_only` / `write_tx` +
      `linkbench_mix` WRITEPHASE → `diagnose.lever` (select/range/
      mix 70% read = `get_path`; write_tx = write lever). — status: `done`
- [x] **P2.11** Surreal `tx_get` / `tx_put` / `tx_rmw` / `tx_scan` /
      `tx_batch` / `tx_rmw_mc8` WRITEPHASE → `diagnose.lever`
      (get/scan = `get_path`; put/rmw/batch = write lever). — status: `done`
- [x] **P2.12** Nebula `get_neighbors` / `insert_edge` WRITEPHASE →
      `diagnose.lever` (neighbors = `get_path`; insert = write lever).
      — status: `done`
- [x] **P2.13** Streaming `flink_window_state` / `kafka_changelog_flush`
      WRITEPHASE → `diagnose.lever` (window = `get_path` mix;
      changelog = write lever). — status: `done`
- [x] **P2.14** Ceph `bluestore_omap_write` / `bluestore_omap_read`
      WRITEPHASE → `diagnose.lever` (read = `get_path`; write = write
      lever). — status: `done`
- [x] **P2.15** Solana `shred_append` / `trailing_read` WRITEPHASE →
      `diagnose.lever` (trailing = `get_path`; shred = write lever).
      — status: `done`
- [x] **P2.16** Arango `doc_crud` / `traversal` WRITEPHASE →
      `diagnose.lever` (traversal = `get_path`; crud mix 70% read =
      `get_path`). — status: `done`
- [x] **P2.17** Venice `fanout_get` / Rockstore `widecol_rw` WRITEPHASE →
      `diagnose.lever` (fanout = `get_path`; widecol mix 50% read =
      `get_path`). — status: `done`
- [x] **P2.18** Oxigraph `spo_lookup` / `triple_put` WRITEPHASE →
      `diagnose.lever` (lookup = `get_path`; triple = write lever).
      — status: `done`
- [x] **P2.19** RocksAPI `mixgraph_like` / `wbwi` / `compaction_filter` /
      `ingest_sst` WRITEPHASE → `diagnose.lever` (wbwi = `get_path`;
      compact/ingest = write lever). — status: `done`
- [x] **P2.20** `ycsb_c_big` WRITEPHASE → `diagnose.lever` (100% uniform
      get over 2^20 = `get_path`). — status: `done`
- [x] **P2.21** scale `probe_hit` p50 ns → `classify_get` vs 0176
      clock (always; published p50, not the later `get_hit` mean).
      — status: `done`
- [x] **P2.22** scale `hydrate` WRITEPHASE → `diagnose.lever`
      (ingest batches; write lever, not `get_path`). — status: `done`
- [x] **P2.23** `BALANCE_SHAPES` inclui `ycsb_b_mc4` (rung 95% get
      já no harness RFC-0163; faltava no gate de engine). — status: `done`
- [x] **P2.24** scale `settle` WRITEPHASE → `diagnose.lever`
      (compact/ingest; write lever, not `get_path`). — status: `done`
- [x] **P2.25** `pedra diagnose write|get|probes|balance` imprime
      JSON object (`{"lever":…}` / `{"class":…}` / `{"admits":…}`).
      — status: `done`
- [x] **P2.26** compare lê `{"lever":…}` de stdout CLI (sem
      `benches[]`). — status: `done`
- [x] **P2.27** compare lê `{"class":…}` de `pedra diagnose get|probes`.
      — status: `done`
- [x] **P2.28** compare lê `{"admits":0|1}` de `pedra diagnose balance`.
      — status: `done`
- [x] **P2.29** `pedra diagnose get` JSON inclui relógio 0176
      (`measured_ns`, `best`, `happy`, `worst`, `as_is` + `class`).
      — status: `done`
- [x] **P2.30** `pedra diagnose probes` JSON inclui `per_get` /
      `p_best` + `class`. — status: `done`
- [x] **P2.31** `WriteDiagnosis::json_object` inclui `gap_ns` /
      `timed_ns` (unattributed = gap−timed). — status: `done`
- [x] **P2.32** `pedra diagnose balance` JSON inclui `shapes` (gate
      0182). — status: `done`
- [x] **P2.33** `predict_get_bottleneck` / `predict_probes_bottleneck`
      classificam o walk as-is a `n` **sem correr um get**. CLI
      `pedra diagnose get --keys N --ram R` (sem `--measured-ns`).
      Teste `predict_get_bottleneck_1b_is_walk_without_runtime`.
      — status: `done`
- [x] **P2.34** Preditor por **probes** não µs: `walk_distinguishable`
      (\(N>2P\)); 1M = `indistinguishable`; 10M = `as_is_walk`.
      `scale_forecast_with` bytes/entry; `predict_write` 1c fd-ceiling /
      mc grouping. CLI `--bytes-per-key`; `diagnose write --clients N`
      sem fases. Teste `predict_get_bottleneck_uses_probes_not_wall_clock`.
      — status: `done`
- [x] **P2.35** Relógio composicional `GetWork × MachineSpec` (spec
      Intel 4 GHz: L1/L2/L3/DRAM/`pread`). Trabalho discreto exacto
      (bloom `k`, `⌈log₂ n_blocks⌉`, FNV bytes, YCSB `ycsb/{i:06}`).
      `--cache happy|capacity|cold`. Envelope 0176 \(P\cdot\tau\)
      fica; este é o lower bound (sem glue/η/OOO). Testes
      `composed_does_not_charge_disk_on_bloom_reject`,
      `happy_is_faster_than_capacity_is_faster_than_cold`,
      `as_is_walk_is_bloom_bound_when_hot`.
      — status: `done`
- [x] **P2.36** `COMPARE_SHAPES` inclui `ycsb_b_mc4` (já em
      `BALANCE_SHAPES` / `run_clients`; compare só itera COMPARE).
      Teste `compare_shapes_keep_official_16_prefix`. — status: `done`
- [x] **P2.37** Relógio WRITE estático nomeia mais que `grouping` vs
      `fd_ceiling`: `async_wal` (1c same-class), `fd_ceiling` (`--sync 1`),
      `grouping` → `next=lock_hold`, grouping pago → `lock_hold`,
      n≥16 → `lock_convoy`, `read_pct≥40` → `get_path`. Diagnose
      medido: `lock_wait` n=2–8 grouping pago = `lock_hold`, não
      convoy. CLI honra `--read-pct --sync --avg-group`. Testes
      `predict_write_mix_names_the_four_cells`,
      `mc4_lock_wait_grouping_paid_is_lock_hold`. — status: `done`
- [x] **P2.38** Crescimento não-linear: `growth=` `one_barrier` /
      `amortize` / `serial_cs` / `convoy_collapse` / `get_bound`. Get:
      `log_n` / `ram_wall` / `walk_all`. Teste
      `predict_write_growth_is_not_linear_in_n`. — status: `done`
- [x] **P2.39** `ycsb_c_mc4` (100% get, 4 clients) in `COMPARE_SHAPES`,
      `BALANCE_SHAPES`, and `run_clients`. Official 16 is 1c C only.
      Test `rfc0184_ycsb_c_mc4_in_compare`. Darwin DIAG vs Rocks
      `sync=false`: `ratio=0.909` (4.71 M / 5.18 M QPS) — named loss
      kept. Not Linux cartaz. — status: `done`
- [x] **P2.40** `qs_hot_get_mc4` (99% get hot 10%, 1% batch, 4 clients)
      in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and `run_qs_clients`.
      Test `rfc0184_qs_hot_get_mc4_in_compare`. Darwin DIAG vs Rocks
      `sync=false`: `ratio=0.719` (2.09 M / 2.90 M QPS) — named loss
      kept. p50 0.7 vs 0.8 µs; hole is tail. Not Linux cartaz. — status: `done`
- [x] **P2.41** `qs_neg_lookup_mc4` (100% miss get, 4 clients) in
      `COMPARE_SHAPES`, `BALANCE_SHAPES`, and `run_qs_clients`.
      Test `rfc0184_qs_neg_lookup_mc4_in_compare`. Darwin DIAG vs Rocks
      `sync=false`: `ratio=0.808` (4.34 M / 5.37 M QPS) — named loss
      kept. p50 0.9 vs 0.6 µs. Not Linux cartaz. — status: `done`
- [x] **P2.42** `qs_batch_write_mc4` (every op one WriteBatch of
      `cfg.batch` puts, 4 clients) in `COMPARE_SHAPES`, `BALANCE_SHAPES`,
      and `run_qs_clients`. Test `rfc0184_qs_batch_write_mc4_in_compare`.
      Darwin DIAG vs Rocks `sync=false`: `ratio=1.021` (32.7 k / 32.0 k
      QPS) — tied on a load-12 host, not a quiet win, Pedra max 147 ms
      vs Rocks 17 ms. Not Linux cartaz. — status: `done`
- [x] **P2.43** `rockset_hybrid_mc4` (ingest WriteBatch + point get, 4
      clients) in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and
      `run_rockset_clients`. Test `rfc0184_rockset_hybrid_mc4_in_compare`.
      Darwin DIAG vs Rocks `sync=false`: `ratio=0.543` (29.0 k / 53.3 k
      QPS) — named loss kept. 1c was 0.811. Not Linux cartaz.
      — status: `done`
- [x] **P2.44** `yugabyte_docdb_rmw_mc4` (70% overlay-get / 30% RMW, 4
      clients) in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and
      `run_yugabyte_clients`. Test `rfc0184_yugabyte_docdb_rmw_mc4_in_compare`.
      Darwin DIAG vs Rocks `sync=false`: `ratio=0.952` (372 k / 390 k
      QPS) — named loss kept. 1c was 0.732. Not Linux cartaz.
      — status: `done`
- [x] **P2.45** `venice_fanout_get_mc4` (32 point-gets / op, 4 clients)
      in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and `run_venice_clients`.
      Test `rfc0184_venice_fanout_get_mc4_in_compare`. Darwin DIAG vs
      Rocks `sync=false`: `ratio=0.735` (102 k / 139 k QPS) — named
      loss kept. Not Linux cartaz. — status: `done`
- [x] **P2.46** `kvrocks_get_mc4` (redis-benchmark GET, 4 clients) in
      `COMPARE_SHAPES`, `BALANCE_SHAPES`, and `run_kvrocks_get_clients`.
      Test `rfc0184_kvrocks_get_mc4_in_compare`. Darwin DIAG vs Rocks
      `sync=false`: `ratio=1.205` (5.32 M / 4.42 M QPS). Not Linux
      cartaz. — status: `done`
- [x] **P2.47** `myrocks_point_select_mc4` (sysbench oltp_point_select,
      4 clients) in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and
      `run_myrocks_clients`. Test `rfc0184_myrocks_point_select_mc4_in_compare`.
      Darwin DIAG: `ratio=0.430` (2.12 M / 4.94 M QPS) — named loss.
      Timed is 100% GET (seed async); JSON suite tag is host-default,
      not a sync-peer win. p50 0.5 vs 0.7 µs; hole is tail (p99 28 vs
      1.8 µs). Not Linux cartaz. — status: `done`
- [x] **P2.48** `nebula_get_neighbors_mc4` (GO 1-hop prefix scan, 4
      clients) in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and
      `run_nebula_clients`. Test `rfc0184_nebula_get_neighbors_mc4_in_compare`.
      Darwin DIAG vs Rocks `sync=false`: `ratio=0.776` (1.56 M / 2.01 M
      QPS) — named loss. p50 0.5 vs 1.8 µs; hole is tail. Not Linux
      cartaz. — status: `done`
- [x] **P2.49** `arango_traversal_mc4` (2-hop prefix scan, 4 clients)
      in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and `run_arango_clients`.
      Test `rfc0184_arango_traversal_mc4_in_compare`. Darwin DIAG vs
      Rocks `sync=false`: `ratio=0.003` (3.44 k / 1.05 M QPS) — named
      loss. p50 2.3 vs 3.5 µs; hole is tail (p99 4.86 vs 0.007 ms,
      wall 58 vs 0.19 s). Rocks 1.05 M 2-hop scans is not collapsed.
      Not Linux cartaz. — status: `done`
- [x] **P2.50** `surreal_tx_get_mc4` (snapshot get + commit, 4 clients)
      in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and `run_surreal_get_clients`.
      Test `rfc0184_surreal_tx_get_mc4_in_compare`. Darwin DIAG
      `ratio=1.364` (364 k / 267 k QPS). Timed is 100% snapshot GET
      (seed async); JSON suite tag is host-default (`peer_sync=true`),
      not a published win vs Rocks default. p50 10 vs 12 µs. Not Linux
      cartaz. — status: `done`
- [x] **P2.51** `oxigraph_spo_lookup_mc4` (SPO point get, 4 clients)
      in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and `run_oxigraph_clients`.
      Test `rfc0184_oxigraph_spo_lookup_mc4_in_compare`. Darwin DIAG vs
      Rocks `sync=false`: `ratio=0.985` (4.28 M / 4.34 M QPS) — named
      loss. Same-class async. p50 0.6 vs 0.8 µs. Not Linux cartaz. —
      status: `done`
- [x] **P2.52** `solana_trailing_read_mc4` (trailing slot scan, 4 clients)
      in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and `run_solana_clients`.
      Test `rfc0184_solana_trailing_read_mc4_in_compare`. Darwin DIAG vs
      Rocks `sync=false`: `ratio=2.414` (2.34 M / 968 k QPS). Same-class
      async. p50 0.3 vs 3.8 µs. Rocks 968 k 25-key scans is not collapsed.
      Not Linux cartaz. — status: `done`
- [x] **P2.53** `kvrocks_scan_mc4` (Redis SCAN COUNT=25, 4 clients)
      in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and `run_kvrocks_scan_clients`.
      Test `rfc0184_kvrocks_scan_mc4_in_compare`. Darwin DIAG vs
      Rocks `sync=false`: `ratio=1.687` (1.75 M / 1.04 M QPS). Same-class
      async. p50 0.4 vs 3.6 µs. Rocks 1.04 M 25-key SCAN is not collapsed.
      Not Linux cartaz. — status: `done`
- [x] **P2.54** `flink_window_state_mc4` (put + window scan, 4 clients)
      in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and `run_flink_clients`.
      Test `rfc0184_flink_window_state_mc4_in_compare`. Darwin DIAG vs
      Rocks `sync=false`: `ratio=0.522` (110 k / 211 k QPS) — named
      loss. Same-class async. p50 32 vs 18 µs. Rocks 211 k put+scan is
      not collapsed. Not Linux cartaz. — status: `done`
- [x] **P2.55** `kafka_changelog_flush_mc4` (WriteBatch ingest, 4 clients)
      in `COMPARE_SHAPES`, `BALANCE_SHAPES`, and `run_kafka_clients`.
      Test `rfc0184_kafka_changelog_flush_mc4_in_compare`. 1c flushes
      every changelog; mc4 times concurrent batch ingest (flush-per-op
      at n=4 is L0 storm). Darwin DIAG vs Rocks `sync=false`:
      `ratio=0.897` (50.8 k / 56.7 k QPS) — named loss. Same-class
      async. p50 72 vs 58 µs. Rocks 57 k batch-ops is not collapsed.
      Not Linux cartaz. — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este ficheiro | 2026-09-07 |
| P0.2 | p0 | kernel + testes 0183 | done | `bench_gap_kernel` | 2026-09-07 |
| P0.3 | p0 | CLI + harness line | done | `pedra diagnose` | 2026-09-07 |
| P0.4 | p0 | get_path + probes + balance_admits | done | `BALANCE_SHAPES`; DIAG board recusa | 2026-09-07 |
| P0.5 | p0 | async ops/CF park O(1) | done | default-over park whole mem; not global-over (0159 P1.3) | 2026-09-07 |
| P0.6 | p0 | ops pin inflight | done | same as 1-op P0.25; drop on Ok | 2026-09-07 |
| P0.7 | p0 | apply_batch/TX async park | done | flush_after_commit_opts; no L0 on no_sync | 2026-09-07 |
| P1.1 | p1 | overwrite_mc4 caixa + diagnose | todo | 0178 P1.3 | 2026-09-07 |
| P1.2 | p1 | compare JSON lever | done | benches[].diagnose.lever; compare ratios | 2026-09-07 |
| P2.1 | p2 | raftlog diagnose.lever | done | 1c + mcN WRITEPHASE → JSON | 2026-09-07 |
| P2.2 | p2 | ycsb 1c diagnose.lever | done | `YcsbRunner::run` WRITEPHASE + read_pct | 2026-09-07 |
| P2.3 | p2 | kvrocks mc50 diagnose.lever | done | n≥16 lock_convoy from WRITEPHASE | 2026-09-07 |
| P2.4 | p2 | scale probe_miss classify_probes | done | COST_TRACE vs P_best; as_is_walk | 2026-09-07 |
| P2.5 | p2 | scale prefix_scan classify_probes | done | scan_sst_probed / op vs P_best | 2026-09-07 |
| P2.6 | p2 | scale get_hit classify_get | done | measured ns vs 0176 best/happy/worst/as_is | 2026-09-07 |
| P2.7 | p2 | scale lookup_100 classify_get | done | loop_ns/100 vs 0176 clock | 2026-09-07 |
| P2.8 | p2 | qs suite diagnose.lever | done | hot/neg get_path; batch WRITEPHASE | 2026-09-07 |
| P2.9 | p2 | kvrocks 1c diagnose.lever | done | get/scan get_path; set WRITEPHASE | 2026-09-07 |
| P2.10 | p2 | myrocks+linkbench diagnose.lever | done | select/range/mix get_path; write_tx WRITEPHASE | 2026-09-07 |
| P2.11 | p2 | surreal diagnose.lever | done | get/scan get_path; put/rmw/batch WRITEPHASE | 2026-09-08 |
| P2.12 | p2 | nebula diagnose.lever | done | neighbors get_path; insert_edge WRITEPHASE | 2026-09-08 |
| P2.13 | p2 | streaming diagnose.lever | done | flink mix get_path; kafka changelog WRITEPHASE | 2026-09-08 |
| P2.14 | p2 | ceph omap diagnose.lever | done | read get_path; write WRITEPHASE | 2026-09-08 |
| P2.15 | p2 | solana diagnose.lever | done | trailing get_path; shred WRITEPHASE | 2026-09-08 |
| P2.16 | p2 | arango diagnose.lever | done | traversal get_path; crud mix 70% | 2026-09-08 |
| P2.17 | p2 | venice diagnose.lever | done | fanout get_path; widecol mix 50% | 2026-09-08 |
| P2.18 | p2 | oxigraph diagnose.lever | done | spo get_path; triple WRITEPHASE | 2026-09-08 |
| P2.19 | p2 | rocksapi diagnose.lever | done | wbwi get_path; compact/ingest WRITEPHASE | 2026-09-08 |
| P2.20 | p2 | ycsb_c_big diagnose.lever | done | 100% get 2^20 → get_path | 2026-09-08 |
| P2.21 | p2 | scale probe_hit classify_get | done | p50 ns vs 0176 best/happy/worst/as_is | 2026-09-08 |
| P2.22 | p2 | scale hydrate diagnose.lever | done | WRITEPHASE ingest batches | 2026-09-08 |
| P2.23 | p2 | BALANCE_SHAPES ycsb_b_mc4 | done | 95% get rung no gate 0182 | 2026-09-08 |
| P2.24 | p2 | scale settle diagnose.lever | done | WRITEPHASE compact/ingest | 2026-09-08 |
| P2.25 | p2 | CLI diagnose JSON object | done | write lever; get/probes class; balance admits | 2026-09-08 |
| P2.26 | p2 | compare CLI diagnose JSON | done | extract_cli_diagnose_lever | 2026-09-08 |
| P2.27 | p2 | compare CLI get/probes class | done | extract_cli_diagnose_class | 2026-09-08 |
| P2.28 | p2 | compare CLI balance admits | done | extract_cli_diagnose_admits | 2026-09-08 |
| P2.29 | p2 | CLI get JSON 0176 clock | done | class+best/happy/worst/as_is | 2026-09-08 |
| P2.30 | p2 | CLI probes JSON inputs | done | class+per_get+p_best | 2026-09-08 |
| P2.31 | p2 | diagnose JSON gap/timed ns | done | json_object gap_ns+timed_ns | 2026-09-08 |
| P2.32 | p2 | CLI balance JSON shapes | done | shapes array = BALANCE_SHAPES | 2026-09-08 |
| P2.33 | p2 | predict bottleneck without a get | done | `predict_get_bottleneck`; diagnose get omits measured-ns | 2026-09-07 |
| P2.34 | p2 | probe-class + write clock + bpe | done | 1M indistinguishable; 10M walk; `predict_write` | 2026-09-07 |
| P2.35 | p2 | GetWork × MachineSpec | done | `predict_get_composed`; `--cache happy|capacity|cold` | 2026-09-08 |
| P2.36 | p2 | COMPARE_SHAPES ycsb_b_mc4 | done | 95% get mc4 visível no compare | 2026-09-08 |
| P2.37 | p2 | static write cuts beyond grouping/fd | done | lock_hold/async_wal/get_path/lock_convoy; next= | 2026-09-08 |
| P2.38 | p2 | non-linear growth tokens | done | serial_cs vs amortize vs convoy_collapse; get ram_wall | 2026-09-08 |
| P2.39 | p2 | ycsb_c_mc4 COMPARE+BALANCE | done | 100% get mc4; Darwin DIAG 0.909× named loss | 2026-09-08 |
| P2.40 | p2 | qs_hot_get_mc4 COMPARE+BALANCE | done | QS hot mc4; Darwin DIAG 0.719× named loss | 2026-09-08 |
| P2.41 | p2 | qs_neg_lookup_mc4 COMPARE+BALANCE | done | QS miss mc4; Darwin DIAG 0.808× named loss | 2026-09-08 |
| P2.42 | p2 | qs_batch_write_mc4 COMPARE+BALANCE | done | QS batch-put mc4; Darwin DIAG 1.021 tied, not a quiet win | 2026-09-08 |
| P2.43 | p2 | rockset_hybrid_mc4 COMPARE+BALANCE | done | Rockset ingest+get mc4; Darwin DIAG 0.543× named loss | 2026-09-08 |
| P2.44 | p2 | yugabyte_docdb_rmw_mc4 COMPARE+BALANCE | done | YB DocDB RMW mc4; Darwin DIAG 0.952× named loss | 2026-09-08 |
| P2.45 | p2 | venice_fanout_get_mc4 COMPARE+BALANCE | done | Venice 32-get fanout mc4; Darwin DIAG 0.735× named loss | 2026-09-08 |
| P2.46 | p2 | kvrocks_get_mc4 COMPARE+BALANCE | done | Kvrocks GET mc4; Darwin DIAG 1.205× not Linux cartaz | 2026-09-08 |
| P2.47 | p2 | myrocks_point_select_mc4 COMPARE+BALANCE | done | MyRocks point-select mc4; Darwin DIAG 0.430× named loss; GET path | 2026-09-08 |
| P2.48 | p2 | nebula_get_neighbors_mc4 COMPARE+BALANCE | done | Nebula 1-hop mc4; Darwin DIAG 0.776× named loss | 2026-09-08 |
| P2.49 | p2 | arango_traversal_mc4 COMPARE+BALANCE | done | Arango 2-hop mc4; Darwin DIAG 0.003× named loss | 2026-09-08 |
| P2.50 | p2 | surreal_tx_get_mc4 COMPARE+BALANCE | done | Surreal snapshot-get mc4; Darwin DIAG 1.364; JSON host-default not a win | 2026-09-08 |
| P2.51 | p2 | oxigraph_spo_lookup_mc4 COMPARE+BALANCE | done | Oxigraph SPO get mc4; Darwin DIAG 0.985× named loss | 2026-09-08 |
| P2.52 | p2 | solana_trailing_read_mc4 COMPARE+BALANCE | done | Solana trailing-read mc4; Darwin DIAG 2.414× not Linux cartaz | 2026-09-08 |
| P2.53 | p2 | kvrocks_scan_mc4 COMPARE+BALANCE | done | Kvrocks SCAN mc4; Darwin DIAG 1.687× not Linux cartaz | 2026-09-08 |
| P2.54 | p2 | flink_window_state_mc4 COMPARE+BALANCE | done | Flink window-state mc4; Darwin DIAG 0.522× named loss | 2026-09-08 |
| P2.55 | p2 | kafka_changelog_flush_mc4 COMPARE+BALANCE | done | Kafka changelog mc4; Darwin DIAG 0.897× named loss; no per-op flush | 2026-09-08 |

## Acceptance Criteria

- **Tests:** `rfc0183_1c_blames_wal_not_memtable`;
  `rfc0183_apply_mc4_blames_flush_check`;
  `classify_get_on_as_is_walk_is_not_ok`;
  `ycsb_a_mixed_is_get_path_not_wal`;
  `rfc0182_darwin_board_refuses_single_diag_cut`;
  `rfc0184_async_ops_does_not_write_l0_when_over_limit`
  (L0=0 **e** `commit_inflight=0` após Ok);
  `rfc0184_async_apply_batch_does_not_write_l0_when_over_limit`;
  raftlog 1c/mcN `diagnose.lever` (P2.1);
  ycsb 1c `run()` `diagnose.lever` (P2.2);
  `kvrocks_set_mc50` `diagnose.lever` (P2.3);
  `mc50_bypass_is_lock_convoy`;
  `classify_probes_on_walk_all_is_not_ok`;
  scale `get_hit` `classify_get` (P2.6);
  scale `lookup_100` `classify_get` (P2.7);
  qs suite `diagnose.lever` (P2.8);
  kvrocks 1c `diagnose.lever` (P2.9);
  myrocks+linkbench `diagnose.lever` (P2.10);
  surreal `diagnose.lever` (P2.11);
  nebula `diagnose.lever` (P2.12);
  streaming `diagnose.lever` (P2.13);
  ceph omap `diagnose.lever` (P2.14);
  solana `diagnose.lever` (P2.15);
  arango `diagnose.lever` (P2.16);
  venice `diagnose.lever` (P2.17);
  oxigraph `diagnose.lever` (P2.18);
  rocksapi `diagnose.lever` (P2.19);
  ycsb_c_big `diagnose.lever` (P2.20);
  scale `probe_hit` `classify_get` (P2.21);
  scale `hydrate` `diagnose.lever` (P2.22);
  `BALANCE_SHAPES` `ycsb_b_mc4` (P2.23);
  scale `settle` `diagnose.lever` (P2.24);
  `pedra diagnose` JSON object (P2.25);
  compare CLI `{"lever":…}` (P2.26);
  compare CLI `{"class":…}` (P2.27);
  compare CLI `{"admits":…}` (P2.28);
  `pedra diagnose get` JSON clock (P2.29);
  `pedra diagnose probes` JSON inputs (P2.30);
  `json_object` `gap_ns`/`timed_ns` (P2.31);
  `pedra diagnose balance` JSON `shapes` (P2.32);
  `predict_get_bottleneck` 1B without a get (P2.33);
  `predict_get_composed` happy/capacity/cold (P2.35);
  `predict_write_mix_names_the_four_cells` (P2.37);
  `mc4_lock_wait_grouping_paid_is_lock_hold` (P2.37);
  `composed_does_not_charge_disk_on_bloom_reject`;
  `ycsb_c_all_reads_timed_zero_is_get_path`;
  `rfc0184_diagnosis_json_has_lever`;
  `extract_diagnose_lever_from_bench_object`;
  `rfc0184_qs_batch_write_mc4_in_compare` (P2.42);
  `rfc0184_rockset_hybrid_mc4_in_compare` (P2.43);
  `rfc0184_yugabyte_docdb_rmw_mc4_in_compare` (P2.44);
  `rfc0184_venice_fanout_get_mc4_in_compare` (P2.45);
  `rfc0184_kvrocks_get_mc4_in_compare` (P2.46);
  `rfc0184_myrocks_point_select_mc4_in_compare` (P2.47);
  `rfc0184_nebula_get_neighbors_mc4_in_compare` (P2.48);
  `rfc0184_arango_traversal_mc4_in_compare` (P2.49);
  `rfc0184_surreal_tx_get_mc4_in_compare` (P2.50);
  `rfc0184_oxigraph_spo_lookup_mc4_in_compare` (P2.51);
  `rfc0184_solana_trailing_read_mc4_in_compare` (P2.52);
  `rfc0184_kvrocks_scan_mc4_in_compare` (P2.53);
  `rfc0184_flink_window_state_mc4_in_compare` (P2.54);
  `rfc0184_kafka_changelog_flush_mc4_in_compare` (P2.55).
- **Telemetry / Analytics:** uma linha `diagnose dominant=… lever=…`;
  `benches[].diagnose.lever` no JSON; compare copia para a row.
  scale `get_hit` / `lookup_100` / `probe_hit` imprimem `diagnose get … class=…` (P2.6/P2.7/P2.21).
  peer continua `sync: false`.
- **Documentation:** este RFC; `docs/benchmarks.md` receita.
- **Screenshots:** backend-only.

## Out of scope

- Skiplist. G1 win. Fjall gate. Bake 4 GiB. WARM 100M na caixa.
- Novo harness. Retune \(\tau\) sem célula Linux nova.
