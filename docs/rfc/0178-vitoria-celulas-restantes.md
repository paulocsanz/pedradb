# RFC-0178 — Vitória >1× nas células restantes

**Status:** in-progress
**Updated:** 2026-09-07
**ID:** 0178
**Parents:** [0177](0177-perdas-nomeadas-crescimento-linear.md) (mapa),
[0161](0161-slipstream-scale-1p5x.md),
[0163](0163-anti-overfit-benchmark-breadth.md),
[0167](0167-endurecimento-achados-adversariais.md) P1.3,
[0168](0168-vitoria-por-celula-toda-escala.md),
[0173](0173-bounded-cache-ram-degrade.md)
**Peer:** RocksDB default `sync=false`. Fjall absoluto. G1 não é win.
**Alvo por célula:** ≥1,0× 3-run **ou** (só 4 GiB / física) ceiling
nomeado. Este RFC **ataca**; 0177 só mapeia.

> P0 é local (teste + DIAG Darwin). 3-run na caixa 4 GiB é P1 —
> não assar sem pedido. Não WARM 100M na caixa 4 GiB.

## Background

Células que o utilizador mandou fechar, com o facto (não o slogan):

| célula | número hoje | o que é de verdade |
|---|---|---|
| prefix perna bloom | **0,70×** slipstream 100M (577 vs 404 µs) | mesma perna do bloom 10 bpk; Pedra 459/577/627 µs (variância); **caixa 4 GiB**, bounded-cache |
| prefix 25M→100M | **65.5 → 65.1 µs** (P0.10, 50M→100M) | Era 67→125 skip-WARM. Same-CLI `pedra scale` Darwin flat. 4 GiB 0,70× continua P1.2 |
| get_loop 50M→100M | **432 → 427 µs** (P0.9 r9) | Era 5,39 ms skip-WARM / ~2 ms com WARM+`stats()` walk. r9: settle deixa de clonar 100M valores; pread 16→1,4 µs/file. Darwin 1-run, não vs Rocks, não 4 GiB |
| ycsb_f_mc4 run2 | intra-run **0,766×**; mediana **PASS 1,473×** | Rocks spikeou 90,3 kqps; Pedra estável 66–69 k. Não é Pedra a piorar |
| probe_miss | **0,27×** pós-bloom; skip tombstone **sem** 3-run | O(ficheiros) vazio já saiu (0167). Razão publicável continua 0,27× até a caixa |
| overwrite_mc4 | Darwin [0180](0180-overwrite-mc4-gt1x.md) P0.9 mediana **1,002×** (p42; named loss 0,816). P1.3 = caixa | Isolado 0,45× → adaptive 0,61× → 0180 1,00×. 3/3 quiet ≥1× é 0182 P1.1. Não G1 |
| 1c overwrite | Darwin [0183](0183-teto-apply-serial-e-1c.md) **0,845×** (284 vs 336 k; p50 3,3 vs 2,6 µs) | WAL/CPU. mem 0,14 µs = 4% do p50. 0055 P1.1 **não** despark |
| apply_mc4 | Darwin 0183 **0,478×** (4,92 vs 10,3 k; wall 81 vs 39 s) | flush_check 148 µs/commit; mem = 2,7% do gap. Skiplist continua parked |

Disco vs Fjall (245 vs 222 B/e) e G1 writes **não** estão neste RFC
(0168 P2.1 / Agents.md).

## Problems This Solves

- **Problem:** prefix 0,70× na caixa é scan bounded-cache mais caro
  que o Rocks no mesmo guest.
- **Problem:** 67→125 µs e 432 µs→5,39 ms foram lidos como “não é
  linear”, mas o 100M daqueles findings **pulou o WARM**. O cap 3/4
  RAM + WARM no flush (0168 P2.4) deve achatar **neste host**. Sem
  DIAG, é hipótese.
- **Problem:** f_mc4 run2 <1× intra-run com mediana PASS — 3/3 ≥1×
  ainda não existe.
- **Problem:** probe_miss e overwrite continuam <1× publicáveis.

## Proposed Solution

- **P0 (este host):** provar que prefix/get_loop **não** dobram 25M→100M
  quando o store WARMa (22 GiB < 72 GiB). Teste: scan de uma janela
  settled+disjunta toca O(ficheiros que sobrepõem a janela), não O(SST
  totais).
- **P0 overwrite/f_mc4:** célula mc isolada (DB fresco) — R7; o 0,557×
  in-suite pode ser a suite. Isolado ≥1× ⇒ a célula oficial passa a
  medir assim.
- **P1 (caixa):** 3-run probe_miss pós-0167; prefix 0,70× no mesmo HEAD;
  overwrite isolado; f_mc4 3/3 intra-run ≥1×. Bounded-cache no 4 GiB
  **não** vira hot — get_loop 100M lá continua disk-bound (ceiling
  nomeado, não escondido).
- **P2:** duas curvas na escada (0177 P2.1). Footprint fora.

Não: mmap, `unsafe`, WARM 100M no 4 GiB, chunk 4 MiB, v8, 0175.

## Delivery slices (mandatory)

### P0 — local, sem caixa

- [x] **P0.1** Este RFC + linha em `docs/status.md` — status: `done`
- [x] **P0.2** DIAG 100M Darwin 1-run com WARM-no-flush —
      status: `done` (prefix **42 µs** vs 67 @25M / 125 skip-WARM —
      flat neste host. get_loop **5,50 ms** cliff **não** fechou.
      settle wall 87 s com `settle_parts` compact=0.002/warm=0 —
      flush-in-settle. `findings/2026-09-06-rfc0178-p02-100m/`)
- [x] **P0.3** Teste dois-estados: prefix de uma janela settled
      disjunta sonda O(1) SST, não O(ficheiros de outros prefixos) —
      status: `done`
- [x] **P0.4** Overwrite/f_mc4: `ROCKS_PARITY_MC_FRESH=1` re-seed
      entre shapes mc; teste `rfc0178_mc_fresh_enabled_reads_env`.
      WARM passa a ser **por path** (compact rewrite não deixa SST
      novo frio). Settle do scale é só `compact()` (já faz flush).
      Follow-up: `ConcurrentDb::flush` dropa o ReadGuard **antes**
      de `note_warmed_ssts` (if-let no guard self-deadlockava) —
      status: `done`
- [x] **P0.5** Settle `compact()` re-WARM do conjunto vivo (path-skip
      do flush fica stale após o worker no `compact_gate`). Worker
      `try_lock` para não starvar o compact explícito. Teste
      `rfc0178_explicit_compact_rewarm_after_flush` — status: `done`
      (DIAG r3: settle `warm_bytes=24.5 GiB` em 1.74 s; get_loop **2.76 ms**
      cliff **não** fechou. Não era path-skip.)
- [x] **P0.6** Worker de compact: um job por tick (não `while` no
      `compact_gate`). DIAG r4: settle **90.9 s** / get_loop **5.73 ms**
      — o `while` **não** era os 87 s (um job ainda segura o gate
      durante `job.write()`). status: `done`
- [x] **P0.7** Largar `compact_gate` durante `job.write()`; hydrate
      `flush_no_notify`. DIAG r5: settle **101.6 s**, get_loop **7.18 ms**
      — compact_ns ainda 0.001; a espera não era só o gate no write.
      status: `done`
- [x] **P0.8** Settle `compact_no_flush` (o 2º flush esperava
      `flush_lock` ~90 s). Worker instala com `lock()` — não apaga
      outputs. DIAG r6/r7: get_loop **1.99–2.50 ms** (melhor);
      settle ainda **85 s**. r8: `compact_leveled_wall=1.745s`
      com settle **82.4 s** — compact **não** era os 80 s.
      status: `done`
- [x] **P0.9** `stats()`/`property_int_value` não materializa SST
      inline. `vlog_size_stats` só anda valores quando há vlog.
      Scale settle: um `stats()` (não 7×). DIAG r9: settle
      **1.743 s** (`stats=0.008s`), get_loop **427 µs** (50M 432),
      get_hit **4.4 µs**. Cliff fechou neste host. Não vs Rocks,
      não 4 GiB. Testes
      `rfc0178_stats_without_vlog_skips_sst_value_walk` /
      `rfc0178_stats_with_vlog_still_counts_live_bytes`.
      status: `done`
- [x] **P0.10** DIAG via `pedra scale` (não o binário cru). 50M+100M
      Darwin same-boot: get_loop **403 / 405 µs**, prefix **65.5 /
      65.1 µs**, settle **0.87 / 1.74 s**, `mode=hot`. Teste
      `rfc0178_pedra_scale_parses_entries_and_dir`. Não vs Rocks,
      não 4 GiB. status: `done`
- [x] **P0.11** overwrite_mc4 isolado (`ROCKS_PARITY_ONLY` honrado em
      `run_clients`). Darwin 1-run vs Rocks default: **0,454×**
      (132k vs 292k qps). Isolado não fecha a célula. Não G1, não
      3-run, não 4 GiB. Teste `rfc0178_mc_only_selects_full_mc_name`.
      status: `done`
- [x] **P0.12** Async write-group adaptativo (merge se 2–8 writers;
      50-thread bypass fica). overwrite_mc4 Darwin 3-run mediana
      **0,61×** (era 0,45×); p95 107→45 µs. Ainda named loss.
      Teste `rfc0178_async_merge_adaptive_small_n_only`. status: `done`
- [x] **P0.13** TLS last-get: `touch` em dirt ≤32 (mesmo bound que
      `point_cache.invalidate_many`). P1.5 era n=1; Adaptive n=2–8
      epoch-bumpava `ycsb_a/f` get_path. Teste
      `rfc0178_tls_precise_small_group_does_not_bump_epoch` +
      `tls_precise_invalidate_matches_point_cache_bound`. — status: `done`
- [x] **P0.14** `lookup` com mem viva usa `sst_envelope` (o path packed
      já rejeitava). probe_miss past-hi não bloom-walk. Teste
      `rfc0178_mem_live_envelope_skips_sst_probe_miss`. — status: `done`

### P1 — caixa 4 GiB (pede bake)

- [ ] **P1.1** probe_miss 100M 3-run HEAD+0167 ≥1,0× vs Rocks default
      **ou** ceiling se ainda O(n) — status: `todo`
- [ ] **P1.2** prefix slipstream 100M 3-run ≥1,0× (fecha o 0,70×) —
      status: `todo`
- [ ] **P1.3** overwrite_mc4 isolado 3-run ≥1,0× **na caixa** —
      status: `todo` (Darwin fechou em 0180; 3/3 quiet é 0182 P1.1)
- [ ] **P1.4** ycsb_f_mc4 3/3 intra-run ≥1,0× (não só mediana) —
      status: `todo`

### P2 — contrato de escala

- [x] **P2.1** Escada 1M/10M/25M/50M/100M com `mode=hot|bounded-cache`
      na linha; proibido racionar os dois — status: `done`
      (`pedra scale` 50M/100M Darwin: `mode=hot`; get_loop **403/405 µs**
      flat. `findings/2026-09-07-rfc0178-p010-pedra-scale/`)
- [ ] **P2.2** 50M/100M na caixa 4 GiB **como bounded-cache** (ceiling
      de RAM, número publicado, não “linear”) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + status | done | este ficheiro | 2026-09-06 |
| P0.2 | p0 | DIAG 100M Darwin WARM (prefix/get_loop flat) | done | prefix 42 µs flat; get_loop 5,50 ms cliff fica; `findings/2026-09-06-rfc0178-p02-100m/` | 2026-09-06 |
| P0.3 | p0 | Prefix window O(overlap) não O(all SST) | done | `rfc0178_prefix_window_probes_overlapping_ssts_only` | 2026-09-06 |
| P0.4 | p0 | Isolate mc + WARM por path | done | `ROCKS_PARITY_MC_FRESH`; `rfc0178_compact_new_paths_are_warmed`; scale settle=`compact()` | 2026-09-06 |
| P0.5 | p0 | compact() re-WARM after gate | done | `clear_warmed_ssts` + worker `try_lock`; DIAG r3 get_loop 2.76 ms | 2026-09-07 |
| P0.6 | p0 | Worker one compact job per tick | done | `compat_compact_once` once per poll, not `while`; r4 settle 90.9 s | 2026-09-07 |
| P0.7 | p0 | Drop compact_gate during job.write | done | r5 settle 101.6 s / get_loop 7.18 ms | 2026-09-07 |
| P0.8 | p0 | compact_no_flush after hydrate | done | skip 2nd flush; no discard-delete; r8 compact=1.745s | 2026-09-07 |
| P0.9 | p0 | stats() skip SST walk without vlog | done | r9 settle 1.743s / get_loop 427µs (flat vs 50M) | 2026-09-07 |
| P0.10 | p0 | DIAG via `pedra scale` | done | 50M/100M get_loop 403/405µs prefix 65.5/65.1µs | 2026-09-07 |
| P0.11 | p0 | overwrite_mc4 isolado | done | ONLY honrado; Darwin 0.454× vs Rocks default (named loss) | 2026-09-07 |
| P0.12 | p0 | adaptive async group 2–8 | done | overwrite_mc4 0.61× 3-run (era 0.45×); still named loss | 2026-09-07 |
| P0.13 | p0 | TLS precise n≤32 (get_path) | done | group no longer epoch-bumps zipf last-get | 2026-09-07 |
| P0.14 | p0 | mem-live lookup uses sst_envelope | done | probe_miss past-hi skips SST bloom walk | 2026-09-08 |
| P1.1 | p1 | probe_miss ≥1× 3-run caixa | todo | — | 2026-09-06 |
| P1.2 | p1 | prefix 0,70× → ≥1× caixa | todo | — | 2026-09-06 |
| P1.3 | p1 | overwrite isolado ≥1× caixa | todo | Darwin 0180; 3/3 quiet 0182 P1.1 | 2026-09-07 |
| P1.4 | p1 | f_mc4 3/3 intra-run ≥1× | todo | — | 2026-09-06 |
| P2.1 | p2 | Duas curvas na escada | done | `pedra scale` 50M/100M mode=hot; get_loop 403/405µs | 2026-09-07 |
| P2.2 | p2 | 50M/100M 4 GiB = bounded-cache | todo | — | 2026-09-06 |

## Acceptance Criteria

- **Tests:** `rfc0178_prefix_window_probes_overlapping_ssts_only` —
  muitos SST noutro prefixo; um scan da janela alvo incrementa
  `scan_sst_probed` em ≤ 2 (um ficheiro da janela + folga de nível).
  P0.4: teste de isolate quando existir.
- **Telemetry / Analytics:** DIAG 100M imprime `settle_parts`,
  `ram_line` / `mode=`. `PEDRA_COST_TRACE` no miss em P1.1.
- **Documentation:** este RFC; finding do DIAG P0.2; 0177 permanece
  o inventário. README público só 3-run.
- **Screenshots:** backend-only.
- **4 GiB:** get_loop/prefix 100M **não** passam a ser hot. Vitória
  lá é ≥ Rocks no **mesmo** modo bounded-cache, não aplanar ao 50M
  Darwin.

## Out of scope

- G1 writes como win. Peer `sync=true`. Fjall como gate.
- WARM 100M na caixa 4 GiB. `PEDRA_BULK_CHUNK_BYTES=4MB`.
- v8 / footprint 245 vs 222 (0168 P2.1).
- RFC-0175 / 0176 (draft).
- Tratar o spike do Rocks no f_mc4 run2 como bug do Pedra.
