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
| prefix 25M→100M | **67 → 125 µs** same-harness | janela **fixa** 1000 chaves (`ROUTES_PER_SERVICE`); 2× mais lento a 4× n. 100M **sem** WARM (cap era 3 GiB). Fjall já ~125 µs @25M |
| get_loop 50M→100M | **432 → 427 µs** (P0.9 r9) | Era 5,39 ms skip-WARM / ~2 ms com WARM+`stats()` walk. r9: settle deixa de clonar 100M valores; pread 16→1,4 µs/file. Darwin 1-run, não vs Rocks, não 4 GiB |
| ycsb_f_mc4 run2 | intra-run **0,766×**; mediana **PASS 1,473×** | Rocks spikeou 90,3 kqps; Pedra estável 66–69 k. Não é Pedra a piorar |
| probe_miss | **0,27×** pós-bloom; skip tombstone **sem** 3-run | O(ficheiros) vazio já saiu (0167). Razão publicável continua 0,27× até a caixa |
| overwrite_mc4 | **0,557×** in-suite; isolado Darwin ~ Rocks guest | Isolar primeiro; depois >1× |

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

### P1 — caixa 4 GiB (pede bake)

- [ ] **P1.1** probe_miss 100M 3-run HEAD+0167 ≥1,0× vs Rocks default
      **ou** ceiling se ainda O(n) — status: `todo`
- [ ] **P1.2** prefix slipstream 100M 3-run ≥1,0× (fecha o 0,70×) —
      status: `todo`
- [ ] **P1.3** overwrite_mc4 isolado 3-run ≥1,0× — status: `todo`
- [ ] **P1.4** ycsb_f_mc4 3/3 intra-run ≥1,0× (não só mediana) —
      status: `todo`

### P2 — contrato de escala

- [x] **P2.1** Escada 1M/10M/25M/50M/100M com `mode=hot|bounded-cache`
      na linha; proibido racionar os dois — status: `done`
      (DIAG 100M Darwin: `mode=hot` imprimiu; get_loop **5,46 ms**
      cliff **não** fechou. `findings/2026-09-07-rfc0178-p21-100m/`)
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
| P1.1 | p1 | probe_miss ≥1× 3-run caixa | todo | — | 2026-09-06 |
| P1.2 | p1 | prefix 0,70× → ≥1× caixa | todo | — | 2026-09-06 |
| P1.3 | p1 | overwrite isolado ≥1× caixa | todo | — | 2026-09-06 |
| P1.4 | p1 | f_mc4 3/3 intra-run ≥1× | todo | — | 2026-09-06 |
| P2.1 | p2 | Duas curvas na escada | done | `mode=hot` @100M Darwin; get_loop 5,46 ms cliff fica; `findings/2026-09-07-rfc0178-p21-100m/` | 2026-09-07 |
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
