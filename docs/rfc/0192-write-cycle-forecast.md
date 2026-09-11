# RFC-0192 — Telemetria de contenção e previsão determinística do ciclo de write

**Status:** in-progress (P0.1/P0.4 done e vivos; P0.2/P0.3/P2.1 wiped-pre-commit — forense 2026-09-11; P1.1/P1.2 re-bloqueados)
**Updated:** 2026-09-11
**ID:** 0192
**Parents:** [0176](0176-modelo-matematico-de-escala.md) (GET clock; este RFC é o gémeo de write),
[0189](0189-ciclo-lider-janela-lenta.md) (modelo lock_wait = ciclo dos outros líderes),
[0190](0190-apply-concorrente-despark.md) (guarda/lanes; o cut seguinte depende de re-medir),
[0169](0169-telemetria-permanente-padroes-lentidao.md) (opt-in; este RFC estende WRITEPHASE, não liga default)
**Peer:** RocksDB default `WriteOptions.sync=false`. Darwin = DIAG. Linux 3-run = cartaz.

> **Tese:** o gerador estático (`cut=lock_hold` 500 ns, fire 120) nomeia a
> fn errada. A previsão de write tem de ser a mesma classe que
> `scale_forecast`: aritmética inteira sobre fatias **medidas**, com um
> AS-IS que reproduz o gerador cego. Telemetria nova só conta contenção
> que o split atual não vê (CAS retry, wait por lane, hold do wal.lock).

## Background

- WRITEPHASE já split `wal(enc/wr) mem(guard/mlock/mins) publish lock_wait grp(walk/complete/settle)`.
- O que **não** vê: retries de CAS (`LfStack`, `published_seq`); wait **por lane** (o trigger do skiplist); hold do `wal.lock` vs wait (hoje `lock_wait` é o acquire).
- O diagnose write usado nos fires 120–122 é **constante** (`lock_hold=500`, `as_is=2200`) — independente das fatias. Por isso skiplist vs `write()`-off-lock vira palpite.
- Linux quieto (0189 P0.1): `guard=2.23` era o max estrutural; `wr=0.89` ainda dentro do mutex; `mlock=0.38`. Depois do 0190 o cartaz não re-mediu.

## Problems This Solves

- **Problem:** `name_cut` estático sempre diz `lock_hold`.
- **Problem:** `lock_wait` é derivado da CS; cortá-lo sem nomear o dono da CS (syscall `write`, guarda, lane) é o método que falhou.
- **Problem:** skiplist TCB (0190 P1.1) não tem oráculo de colapso de lane (zipf → 1 de 8).

## Proposed Solution

1. Kernel `write_cycle_kernel.rs` (inteiro, sem I/O): CS serial, `lock_wait` previsto `(L−1)/L·CS`, QPS `1e9/cycle`, `name_cut` = max das fatias **estruturais** (não `lock_wait`). AS-IS = sempre `LockHold`.
2. Contadores opt-in no mesmo latch `PEDRA_WRITE_PHASE_STATS`: CAS fails; wait/contended por lane (Instant **só** no `try_write` miss); hold do wal.lock.
3. A linha WRITEPHASE imprime `cut=` + `qps_hat` + `off_wr_qps_hat` + `lane_c=` a partir do kernel, não de um número cravado.

## Delivery slices (mandatory)

### P0 — kernel + dump (útil sozinho)

- [x] **P0.1** `write_cycle_kernel`: `name_cut` / `name_cut_as_is` / CS / QPS / lock_wait previsto / colapso de lane — testes pinados nos ns do Linux quieto 0189 P0.1 — status: `done`
- [x] **P0.1** `write_cycle_kernel`: `name_cut` / `name_cut_as_is` / CS / QPS / lock_wait previsto / colapso de lane — testes pinados nos ns do Linux quieto 0189 P0.1 — status: `done` (vivo no tree; 19/19 verdes 2026-09-11)
- [ ] **P0.2** Telemetria: `lf_cas` / `pub_cas` / `lane_c` / `wal_hold`; linha WRITEPHASE chama o kernel — status: `wiped-pre-commit` (forense 2026-09-11, `findings/2026-09-11-wipe-forense/`: a instrumentação fine-slice + o render via kernel foram verificados em working tree 2026-09-10 e jamais commitados; tree vivo tem as 6 fatias RFC-0159. Reconstruir é pré-requisito do P1.1)
- [ ] **P0.3** `pedra scale-model write` imprime `write_cycle_forecast` (mesmo kernel; AS-IS = `lock_hold`) — status: `wiped-pre-commit` (subcomando CLI inexistente no tree vivo; kernel P0.1 vivo)
- [x] **P0.4** Tier calibrado + rótulo de teto (2026-09-10, `findings/2026-09-10-write-forecast-why-it-missed.md`): o hat determinístico errou +1414…+2194‰ nas pernas isoladas por (1) conflação teto↔previsão (`1e9/ciclo` de um pipeline serial ≠ `L/média` do cliente; no pin quieto o "teto" fica −435‰ ABAIXO do medido), (2) variabilidade ausente (caudas p99/p50 52–65× ⇒ scv 17–24, amplificação ~9–12×), (3) trabalho fora-de-fase (43% da média r2), (4) pernas fora do protocolo STOP/CONT warm10 (fator ×7 só na Pedra), (5) pin vintage. Conserto: `calibrated_forecast(leaders, p50, p99, cut_shift, measured)` — lognormal inteira (Q=2^20, ln atanh, exp Taylor, clamp 1024×), `mean_hat = p50·e^{σ̂²/2}`, `qps_hat = L·1e9/mean_hat`, erro P1.2, e **ganho de corte projetado na média calibrada** (0193: +33‰, não +60%); renders carregam `tier=ceiling`/`tier=forecast`; pernas 03:32Z pinadas ±102‰ — status: `done`

### P1 — o que o kernel decide

- [ ] **P1.1** Perna Linux quieta do 0190: `name_cut` pós-guarda; se `WalWrite` ⇒ 0189 P1.2; se `MemLock` + `lane_collapsed` ⇒ 0190 P1.1 TCB — status: `blocked` (re-bloqueado 2026-09-11, razão NOVA: gate aberto, mas a wiring desta fatia — as 8 fatias finas enc/wr, guard/mlock/mins, grp walk/complete/settle + o render WRITEPHASE chamando o kernel — foi apagada pelo `git reset --hard` de sessão paralela em 2026-09-10 23:49; tree vivo 2026-09-11 tem `WritePhaseStats` com as 6 fatias RFC-0159 e render pré-kernel, kernel intacto 18/18 @ `b959428a`. Reconstruir o P0.2 é fatia própria deste RFC e pré-requisito do re-pin)
- [ ] **P1.2** `off_wr_qps_hat` vs QPS medido na mesma perna (erro do modelo nomeado, não escondido) — status: `blocked` na perna (re-bloqueado 2026-09-11, mesma razão do P1.1: sem a wiring re-construída não há perna; a aritmética `qps_hat_error_permille` segue aterrada no kernel, 3 testes)

### P2 — polish

- [ ] **P2.1** Histogramas de lane no JSON do compare — status: `wiped-pre-commit` (forense 2026-09-11: `lane_histogram` ausente do tree e de todo o histórico em `crates/`; verificado em working tree 2026-09-10)
- [ ] **P2.2** none yet

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | kernel name_cut + CS + QPS | done (vivo; 19/19 verdes 2026-09-11) | `write_cycle_kernel.rs` | 2026-09-11 |
| P0.2 | p0 | CAS / lane / wal_hold no WRITEPHASE | wiped-pre-commit (forense 2026-09-11) | `2026-09-11-wipe-forense/` | 2026-09-11 |
| P0.3 | p0 | CLI write forecast | wiped-pre-commit (forense 2026-09-11) | `2026-09-11-wipe-forense/` | 2026-09-11 |
| P0.4 | p0 | tier calibrado (lognormal p50/p99) + rótulo `tier=ceiling` | done | `calibrated_forecast` + `2026-09-10-write-forecast-why-it-missed.md` | 2026-09-10 |
| P1.1 | p1 | Linux quieto decide o corte | blocked (wipe da wiring própria, datado 2026-09-11; gate aberto) | verificado in-tree: 6 fatias RFC-0159; kernel intacto `b959428a` | 2026-09-11 |
| P1.2 | p1 | erro do modelo vs medido | blocked (perna; mesma razão; kernel aterrado nos dois tiers) | `qps_hat_error_permille` | 2026-09-11 |
| P2.1 | p2 | lane hist no JSON | wiped-pre-commit (forense 2026-09-11) | `2026-09-11-wipe-forense/` | 2026-09-11 |

## Acceptance Criteria

- **Tests**
  - `rfc0192_linux_quiet_names_guard` — ns 0189 P0.1 ⇒ `MemGuard`; AS-IS ⇒ `LockHold`.
  - `rfc0192_after_guard_names_wal_write` — guard=0 ⇒ `WalWrite`.
  - `rfc0192_one_leader_lock_wait_is_zero` — L=1 ⇒ predicted wait 0.
  - `rfc0192_lane_collapse_is_max_gt_twice_fair` — zipf vs uniforme.
  - `rfc0192_as_is_always_lock_hold` — dente.
  - `rfc0192_forecast_is_the_cli_table` — `write_cycle_forecast` = átomos.
  - `rfc0192_write_cycle_line_uses_kernel` — dump real `ConcurrentDb` ≠ `cut=lock_hold`.
  - `rfc0192_lane_write_counts_contention` — Instant só no `try_write` miss.
  - `rfc0192_pedra_scale_model_write_prints_kernel` — CLI = `render()`.
  - `rfc0192_calibrated_forecast_pins_serial_md_legs` — pernas 03:32Z: 94 288/+80‰, 103 626/−102‰, 110 518/+71‰.
  - `rfc0192_ceiling_error_was_an_order_of_magnitude_worse` — teto antigo ≥13× pior que o calibrado em toda perna.
  - `rfc0192_ceiling_is_not_a_bound_on_quiet_leg` — 125 313 vs 222 125 = −435‰: o "teto" nunca foi teto.
  - `rfc0192_deterministic_tail_is_identity` — p99=p50 ⇒ mult 1000/scv 0/amp 500‰.
  - `rfc0192_tail_clamps_at_1024x` — saturação declarada, sem pânico.
  - `rfc0192_ln_fp_pins` — ln 2/10/64,8 inteiros (Q=2^20).
  - `rfc0192_cut_gain_forecasts_on_calibrated_mean` — corte 0193 = +33‰, não +60%.
  - `rfc0192_calibrated_render_carries_tier_and_error` — dump do tier calibrado.
- **Telemetry / Analytics**
  - `PEDRA_WRITE_PHASE_STATS=1` imprime `cut=` do kernel. Sem o env, zero Instant no insert (só `try_write` miss conta wait).
- **Documentation**
  - Este RFC; kernel documenta o modelo `(L−1)/L·CS` como **teto**
    estrutural de ranqueamento (`tier=ceiling`) — nunca previsão de QPS
    cartaz e nem bound sob L>1 (P0.4); a previsão comparável ao bench é o
    `tier=forecast` calibrado (lognormal p50/p99, Little L/média).
- **Screenshots**
  - Backend-only — n/a.

## Out of scope

- Ligar telemetria por default (0169).
- Skiplist / write-off-lock (o kernel **nomeia**; 0189/0190 executam).
- GET clock (0176). G1 1c fd-ceiling.
