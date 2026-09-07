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

### P1 — Linux

- [ ] **P1.1** `overwrite_mc4` isolado na caixa + diagnose (0178 P1.3) —
      status: `todo`
- [ ] **P1.2** compare JSON inclui `diagnose.lever` — status: `todo`

### P2 — polish

- [ ] **P2.1** none yet — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este ficheiro | 2026-09-07 |
| P0.2 | p0 | kernel + testes 0183 | done | `bench_gap_kernel` | 2026-09-07 |
| P0.3 | p0 | CLI + harness line | done | `pedra diagnose` | 2026-09-07 |
| P1.1 | p1 | overwrite_mc4 caixa + diagnose | todo | 0178 P1.3 | 2026-09-07 |
| P1.2 | p1 | compare JSON lever | todo | — | 2026-09-07 |
| P2.1 | p2 | none yet | todo | — | 2026-09-07 |

## Acceptance Criteria

- **Tests:** `rfc0183_1c_blames_wal_not_memtable`;
  `rfc0183_apply_mc4_blames_flush_check`;
  `classify_get_on_as_is_walk_is_not_ok`.
- **Telemetry / Analytics:** uma linha `diagnose dominant=… lever=…`;
  peer continua `sync: false`.
- **Documentation:** este RFC; `docs/benchmarks.md` receita.
- **Screenshots:** backend-only.

## Out of scope

- Skiplist. G1 win. Fjall gate. Bake 4 GiB. WARM 100M na caixa.
- Novo harness. Retune \(\tau\) sem célula Linux nova.
