# RFC-0180 — overwrite_mc4 ≥1× vs Rocks default

**Status:** in-progress
**Updated:** 2026-09-07
**ID:** 0180
**Parents:** [0178](0178-vitoria-celulas-restantes.md) P0.11–P0.12
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
G1 não é win. Fjall absoluto, não gate.

> P0 é local (teste + DIAG Darwin isolado). 3-run na caixa 4 GiB é P1 —
> não assar sem pedido.

## Background

- Isolated `deps_cache_overwrite_mc4` (P0.11 `ONLY` + 4 clients +
  `MC_FRESH`): drop-in same-class vs Rocks default.
- P0.12 adaptive async group (merge se 2–8 writers): Darwin 3-run
  mediana **0,61×** (161 k vs 227–274 k). p95 107→45 µs. p50 ainda
  **20 µs vs 12 µs**.
- O convoy do write-lock já não é o resto. O resto é custo por op
  (encode, vlog idle, WAL `write()`, grupo).

## Problems This Solves

- **Problem:** overwrite_mc4 publicável continua <1× no drop-in
  (0,61×). Isolado não era só a suíte.
- **Problem:** p50 20 vs 12 µs — o grupo amortiza o rabo e paga
  latência no mediano.

## Proposed Solution

Cortar o custo por op no put async 1-op a 4 clientes, sem desligar
durabilidade nem o bypass a 50 threads. Cada fatia P0 é uma hipótese
medida no mesmo harness isolado.

## Delivery slices (mandatory)

### P0 — local, até ≥1,0×

- [x] **P0.1** Este RFC — status: `done`
- [ ] **P0.2** Baseline isolado 1-run vs Rocks `SYNC=0` (confirmar
      ~0,61×) — status: `doing`
- [ ] **P0.3** Não preparar vlog no put inline (sem pending) —
      status: `todo`
- [ ] **P0.4** overwrite_mc4 3-run mediana ≥1,0× vs Rocks default —
      status: `todo`

### P1 — caixa 4 GiB (pede bake)

- [ ] **P1.1** overwrite_mc4 isolado 3-run ≥1,0× na caixa — status: `todo`

### P2 — polish

- [ ] **P2.1** none yet — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este ficheiro | 2026-09-07 |
| P0.2 | p0 | Baseline isolado | doing | — | 2026-09-07 |
| P0.3 | p0 | Skip idle vlog on inline put | todo | — | 2026-09-07 |
| P0.4 | p0 | overwrite_mc4 ≥1× 3-run | todo | — | 2026-09-07 |
| P1.1 | p1 | 3-run caixa | todo | — | 2026-09-07 |
| P2.1 | p2 | none yet | todo | — | 2026-09-07 |

## Acceptance Criteria

- **Tests:** unit test that drives the shipped policy/path (same style
  as `rfc0178_async_merge_adaptive_small_n_only`).
- **Telemetry / Analytics:** isolated overwrite_mc4 JSON + compare;
  peer `"sync": false`.
- **Documentation:** this RFC; 0178 keeps the inventory row.
- **Screenshots:** backend-only.

## Out of scope

- G1 write row as win. Peer `sync=true`. Fjall as gate.
- Bake 4 GiB box. mmap, `unsafe`, chunk 4 MiB, RFC-0175.
- ycsb_f_mc4 3/3 on the box.
