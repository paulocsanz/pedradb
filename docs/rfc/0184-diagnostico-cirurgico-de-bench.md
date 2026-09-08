# RFC-0184 — Diagnóstico cirúrgico de um cell de bench

**Status:** in-progress
**Updated:** 2026-09-07
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
  `ycsb_c_all_reads_timed_zero_is_get_path`;
  `rfc0184_diagnosis_json_has_lever`;
  `extract_diagnose_lever_from_bench_object`.
- **Telemetry / Analytics:** uma linha `diagnose dominant=… lever=…`;
  `benches[].diagnose.lever` no JSON; compare copia para a row.
  scale `get_hit` / `lookup_100` / `probe_hit` imprimem `diagnose get … class=…` (P2.6/P2.7/P2.21).
  peer continua `sync: false`.
- **Documentation:** este RFC; `docs/benchmarks.md` receita.
- **Screenshots:** backend-only.

## Out of scope

- Skiplist. G1 win. Fjall gate. Bake 4 GiB. WARM 100M na caixa.
- Novo harness. Retune \(\tau\) sem célula Linux nova.
