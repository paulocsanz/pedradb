# RFC-0197 — Curva de razão same-class por escala (ratio_hat determinístico)

**Status:** terminal (P0.1–P0.3 done; P2.1 deferred com âncoras GET datadas
registradas no kernel; meters P1/P2.2 blocked no gate datado — re-check
22:24 pós-restart do OrbStack, `2026-09-10-host-gate-blocked-meter.md`)
**Next:** desbloquear o host gate ⇒ P1.1 meter 100M write (confirma/refuta o 0,417 hat)
**Updated:** 2026-09-10
**ID:** 0197
**Parents:** [0185](0185-coluna-a-dropin-1x-tudo.md) (o alvo-produto: coluna A ≥1× em **tudo**),
[0196](0196-meter-first-publish-unification.md) (meter-first; este RFC dá a cada meter a sua previsão por escala),
[0192](0192-write-cycle-forecast.md) (o kernel de write que a curva reusa),
[0176](0176-modelo-matematico-de-escala.md) (o gêmeo GET: `scale_kernel`, cold-fraction e `warm_cap`),
[0041](0041-2x-rocks-default.md) (o piso registrado da coluna same-class)
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`). G1 1c não é win.
Darwin = DIAG. Linux 3-run quieto = cartaz. Previsão é **hat** com erro nomeado — nunca cartaz.
Nota (0192 P0.4, 2026-09-10): o `qps_hat` por escala desta curva é família do
**teto** (`1e9/ciclo`) — serve para o déficit de ciclo e o `cut_to_cross`;
ganho de QPS **realizado** projeta na média calibrada (lognormal p50/p99,
`findings/2026-09-10-write-forecast-why-it-missed.md`).

> **Tese:** "ganhar 100% async-vs-async em toda escala" sem previsão por escala é
> palpite: o smoke 15/15 (1024 records) convive com 0,557× em 25M porque a razão
> **cai com a fração fria do store** e nenhum instrumento atual a modela. A curva
> `ratio_hat(records)` é aritmética inteira sobre (a) âncoras **datadas** por
> escala, (b) o `warm_cap`/cold-fraction do `scale_kernel` vivo e (c) o ranking
> de fatias do `write_cycle_kernel` vivo — sem nova fórmula de engine. Ela diz,
> **antes** do meter: onde a razão cruza 1,0, quanto ns de déficit existe em
> cada escala, e qual fatia nomeada cobre o déficit. O AS-IS é a curva plana
> (sem termo de escala) — a cegueira exata que produziu "15/15 passa".

## Background — a escada datada da família write (`deps_cache_overwrite_mc4`, 4 clientes, payload 100, peer `sync=false`)

| escala | store (245 B/op) | cold ‰ (`warm_cap` 4 GiB = 3 GiB) | pedra | rocks | ratio | rótulo/data |
|---|---:|---:|---:|---:|---:|---|
| 100k | 24,5 MB | 0 | 236 k | 266 k | 0,887 | Darwin DIAG (mapa otimizar, floor-cut) |
| 2M | 490 MB | 0 | 209 k | ≳263 k | 0,795 | Darwin DIAG (leftover HOT) |
| 15M | 3,68 GB | 123 | 174 k | 264 k | 0,66 | Darwin DIAG (primeira medição) |
| 25M | 6,13 GB | 474 | 142 959 | 256 780 | **0,557** | **Linux cartaz** (2026-09-10 r3 min-of-3, `2026-09-10-take-all-reversal-takecell.md`) |
| 100M | 24,5 GB | 868 | — | — | **0,417 hat** | **previsão deste RFC — METER** |

Fatos que a escada nomeia:

- **O ciclo do Rocks é plano**: 3 759–3 894 ns/op de 100k a 25M (spread 135 ns
  ≈ ±18‰). Modelar `rocks_cycle` como constante datada (média 3 810 ns) é uma
  hipótese nomeada, refutável no meter 100M (o LRU de página do kernel do
  Rocks pode curvar).
- **O ciclo do Pedra cresce com o cold**: 4 237 → 4 784 → 5 747 → 6 995 ns.
  Fit inteiro (mínimos quadrados, 4 pontos): `base ≈ 4 674 ns` +
  `≈ 5,14 ns por cold-permille`; resíduo máx **93‰** (100k, cross-box — 3
  âncoras Darwin + 1 Linux, declarado).
- **O erro do modelo de CS é grande e nomeado**: no isolado 1024×10k o
  `lock_wait` medido é 16,7–18,5 µs/op contra o hat `(L−1)/L·CS ≈ 1,5 µs`
  (cauda p99) — o hat de CS **não** é ciclo fim-a-fim. Por isso a curva ancora
  em ciclos medidos datados, e o CS entra só na decomposição do corte.
- **Extrapolação 100M**: cycle_hat 9 134 ns ⇒ ratio_hat **417‰ (0,417×)** —
  buraco **previsto**, pior que o prefix 100M (0,70×). Se o meter confirmar, o
  100M write é o rank 2 da coluna.

## Problems This Solves

- **Problem:** o gate oficial (smoke 1024 records) passa 15/15 enquanto toda a
  família write perde em escala — nenhuma previsão liga os dois.
- **Problem:** cada meter é cego à escala vizinha (10k/15M/25M medidos em
  protocolos e datas diferentes, sem curva que os una).
- **Problem:** "qual corte ganha esta escala" não tem resposta determinística
  hoje: déficit por escala não é decomposto em termo de disco × gap de base.
- **Problem:** a regra do skill (meter o **mesmo tamanho** do buraco) não tem
  instrumento que diga qual o próximo tamanho a medir.

## Proposed Solution

1. **Kernel puro `ratio_curve_kernel.rs`** (inteiro, sem I/O, sem float):
   `cold_permille(store, cap)` (reusa o `warm_cap_bytes` do `scale_kernel`);
   fit de mínimos quadrados inteiro sobre âncoras datadas
   (`fit_write_family_curve`); `cycle_hat(cold)`; `ratio_hat_permille`;
   `cut_to_cross(cycle, rocks_cycle)` → déficit decomposto
   (`disk_ns` vs `base_gap_ns`) + `covering_cuts` no ticket view do
   `write_cycle_kernel` (publish 820 + mem_insert 580 cobrem o gap de base em
   100M); AS-IS = curva plana (sem termo de escala — o dente do smoke).
2. **CLI `pedra scale-model ratio --ram R --scales n,n,...`** imprime a curva
   por escala com: cold, cycle_hat, qps_hat, ratio_hat, o medido datado
   (quando existe) e o `error_permille` do kernel 0192 — mesma perna que
   `scale-model write`.
3. **Ranking com extrapolações nomeadas**: toda célula <1× ganha a sua
   `ratio_hat` nos tamanhos não medidos, rotulada **hat** — o 100M write
   (0,417 hat) entra como buraco previsto a METER, não como número.
4. **P1 meters na escada** (mesmo gate do 0196): 100M write (novo), 15M/25M
   Linux 3-run (re-fit same-box), prefix 100M (0195 P0.4), re-split quieto
   (0196 P0.1). Cada meter devolve `qps_hat_error_permille` e re-ancora a
   curva — a curva é viva, não um pin.
5. **P2 lado GET da curva**: `scale_kernel` já dá o ns/op do Pedra por
   (keys, ram); as âncoras datadas do lado Rocks por escala estão
   **registradas** (`GET_SIDE_ANCHORS_2026_09_10`: 100M em 2 caixas —
   prefix 700‰ cartaz, big-guest 1050‰, point-gets DIAG 1484–1576‰);
   10k/2M/15M/25M GET são deflexão nomeada até os meters P1 medirem.

## Delivery slices (mandatory)

### P0 — curva + CLI + ranking (útil sozinho, aterriza com o gate fechado)

- [x] **P0.1** `ratio_curve_kernel.rs`: `cold_permille`, fit inteiro,
      `cycle_hat`, `ratio_hat_permille`, `cut_to_cross` + decomposição +
      `covering_cuts`, AS-IS plano; âncoras datadas como consts com rótulo;
      testes pinam fit (base/slope/resíduo máx 93‰), extrapolação 100M =
      417‰ **hat**, coberturas e o dente AS-IS — status: `done`
- [x] **P0.2** `pedra scale-model ratio --ram R --scales ...`: imprime a curva
      (hat + medido datado + error_permille) e o `cut_to_cross` por escala;
      teste CLI = render do kernel — status: `done`
- [x] **P0.3** Ranking vivo: 100M write 0,417 hat (METER), ponteiro no 0196,
      linha no `docs/status.md` — status: `done`

### P1 — meters na escada (blocked no mesmo gate; 0196 P0.1 destrava o host)

- [ ] **P1.1** 100M write @4GiB Linux 3-run (novo cartaz que a curva nomeou;
      confirma/refuta o 0,417 hat e a constância do `rocks_cycle`) — status:
      `blocked` (gate datado, `2026-09-10-host-gate-blocked-meter.md`)
- [ ] **P1.2** 15M/25M Linux 3-run same-box → re-fit da curva sem âncoras
      Darwin (cross-box sai do fit) — status: `blocked`
- [ ] **P1.3** prefix 100M (0195 P0.4) + re-split quieto (0196 P0.1) na mesma
      janela; erros `qps_hat_error_permille` de volta pra curva — status:
      `blocked`
- [ ] **P1.4** Eixo clientes: tabela `ratio_hat(L)` reusando o 0192 (mc50
      0,37×; apply_mc4 0,47×) no mesmo CLI — status: `blocked` (precisa o
      re-split para o CS pós-0193 medido; o corte do buraco mc50 em si é
      [0201](0201-cliente-drain-cheio-spin-oversub.md) P0 — wiring destruído
      pelo wipe 2026-09-10 23:49, kernel re-registrado, meter rodando)

### P2 — atrás dos meters

- [ ] **P2.1** Lado GET da curva — **deferred com âncoras registradas
      (2026-09-10)**: `GET_SIDE_ANCHORS_2026_09_10` no kernel + render no
      `scale-model ratio` (3 testes `rfc0197_p21_*` + CLI). Escada datada
      existente: **100M em 2 caixas** — prefix 700‰ (cartaz) + big-guest
      1050‰ (contraste RFC-0195) + point-gets DIAG 3-run vlen=200
      (get_hit 1576‰, get_loop 1484‰, multi_get 1562‰, prefix 1309‰,
      `findings/2026-09-04-win-probe-prefix/`). 10k/2M/15M/25M GET não têm
      medição datada ⇒ **deflexão nomeada** (o próprio render declara a
      escada incompleta); re-ancora pelos meters P1.2/P1.3 quando o gate
      abrir — status: `deferred (1 de 5 escalas; âncoras reais, fit proibido)`
- [ ] **P2.2** Grid B (10–100×, compaction on) re-ancora a curva com compação
      viva — status: `blocked` (carrega 0196 P2.2)

## Ranking — buracos + previsões (número medido datado OU hat rotulado)

| # | célula | número (data, rótulo) | dono/atribuição | ataque | banda |
|---|---|---|---|---|---|
| 1 | apply_mc4 same-class | 0,47× (Linux 0183) | publish epochs vivos (0196 P0.2 condicionada) | 0196 P0 | P0 |
| 2 | overwrite **100M @4GiB** | **0,417 hat** (este RFC; METER) | disco 4 461 ns (84% do déficit 5 324) + base 863 | meter P1.1; cortes 0194/0195 compõem | **P1.1** |
| 3 | prefix 100M @4GiB | 0,70× (cartaz 09-10) | scan bounded `pread` — 0195 aterrizado | meter 0195 P0.4 | P1.3 |
| 4 | overwrite_mc4 25M | 0,557× 3/3 (Linux 09-10) | 0193+0194 aterrizados sem meter | meter 0194 P0.4 | P1.2 |
| 5 | kvrocks_set_mc50 | 0,37× (cartaz) | convoi serial; eixo L | meter 0193 P0.5 | P1.4 |
| 6 | overwrite_mc4 10k/100k | 0,883 min / 0,887 DIAG | base gap quente (publish+mins pós-ticket) | meter + 0196 P0.2 | P1 |
| 7 | ycsb_f_mc4 | 0,766 run2 (09-09) | 0193 aterrizado; rmw restante | meter 0193 P0.5 | P1 |
| 8 | ycsb_b_mc4 | sem Linux 3-run | cauda GET | medir (0196 P1.4) | P1 |

Por que o P0 é a curva e não um corte: o host gate está fechado (load1 ≥ 8,
guest `Unknown host` — `2026-09-10-host-gate-blocked-meter.md`) e o skill
`otimizar` veda engine-cut/meter com o gate falhado; o precedente 0192 é
exatamente isto — kernel determinístico aterriza com o gate fechado, meter
blocked datado. A curva é o instrumento que faz o "100% em toda escala"
ser **decidível**: cada meter volta com erro nomeado e re-ancora.

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | kernel ratio_curve + testes | done | `ratio_curve_kernel.rs` (9 testes `rfc0197_*`) | 2026-09-10 |
| P0.2 | p0 | CLI scale-model ratio | done | `rfc0197_scale_model_ratio_prints_curve` | 2026-09-10 |
| P0.3 | p0 | ranking vivo + ponteiros | done | este RFC + status.md + `Next` no 0196 | 2026-09-10 |
| P1.1 | p1 | meter 100M write (novo) | blocked | gate datado (re-check 22:24) | 2026-09-10 |
| P1.2 | p1 | 15M/25M same-box re-fit | blocked | gate datado (re-check 22:24) | 2026-09-10 |
| P1.3 | p1 | prefix 100M + re-split na janela | blocked | 0195 P0.4 / 0196 P0.1 | 2026-09-10 |
| P1.4 | p1 | eixo clientes ratio_hat(L) | blocked | re-split primeiro | 2026-09-10 |
| P2.1 | p2 | lado GET da curva | deferred | âncoras 100M registradas (2 caixas); 4 escalas sem medição datada | 2026-09-10 |
| P2.2 | p2 | Grid B re-ancora | blocked | 0196 P2.2 | 2026-09-10 |

## Acceptance Criteria

- **Tests**
  - `rfc0197_cold_permille_zero_hot_and_monotone_bounded` — 100k/2M ⇒ 0;
    15M/25M/100M cresce; ≤ 1000.
  - `rfc0197_fit_reproduces_anchors_within_declared_residual` — resíduo máx
    93‰ nos 4 pontos datados (tolerância declarada cross-box 100‰).
  - `rfc0197_extrapolation_100m_is_417_hat` — 417‰ rotulado hat, nunca
    medido.
  - `rfc0197_rocks_cycle_const_is_dated_flat` — 3 810 ns, spread 135 ns.
  - `rfc0197_cut_to_cross_decomposes_disk_and_base` — 100M: déficit 5 324 =
    disco 4 461 + base 863; cobertura publish+mem_insert ≥ base gap.
  - `rfc0197_as_is_curve_is_flat` — sem termo de escala, ratio_hat igual em
    toda escala (a cegueira do smoke 15/15).
  - `rfc0197_refit_at_wrong_ram_breaks_tolerance` — âncoras de 4 GiB
    re-avaliadas com cap de 64 GiB degeneram (slope 0, resíduo 285‰ >
    tolerância): misuse é rejeitado, não clampeado.
  - `rfc0197_scale_model_ratio_prints_curve` — CLI = render do kernel.
- **Telemetry / Analytics**
  - `pedra scale-model ratio` é opt-in por invocação; nenhum default muda.
- **Documentation**
  - Este RFC; `docs/status.md`; ponteiro `Next` no 0196.
- **Screenshots**
  - Backend-only — n/a.

## Out of scope

- Engine cuts neste ciclo (host gate fechado; skill `otimizar` Methods §0).
- CITAR o 0,417 hat como número de célula — é previsão até o P1.1 medir.
- Trocar o peer, o protocolo min-of-3, ou o piso 0041.
- Telemetria default-on (0169); skiplist TCB (0190 P1.1 non-condition);
  grouping/linger/intern/TLS (pagos).
- Curva GET com âncoras inventadas (P2.1 exige medição datada por escala).
