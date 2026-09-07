# RFC-0183 — Teto apply serial e 1c overwrite

**Status:** in-progress
**Updated:** 2026-09-07
**ID:** 0183
**Parents:** [0055](0055-rocks-write-pipeline.md),
[0180](0180-overwrite-mc4-gt1x.md),
[0182](0182-same-boot-write-path-harnesses.md)
**Peer:** RocksDB default `sync=false`. G1 não é win.

> P0 é medir o gap. Concurrent skiplist **não** é P0. RFC-0055 P1
> continua `iff` o número obrigar.

## Background

- Pedra aplica memtable **no write lock**, um insert de cada vez
  (RFC-0055: `allow_concurrent_memtable_write` do Rocks é no-op).
  1c overwrite chão **~0,81×** (p50 2,9 vs 2,4 µs; p48 237 k vs 288 k
  = **0,82×**). O resto é CPU/WAL, não grupo.
- overwrite_mc4 depois do 0180: p50 ainda **13–14 vs 11–12 µs** no
  quieto. QPS 1,00× veio de p95/p999, não de empatar o p50.
- `deps_apply_batch_mc4` p48: **0,47×** (5,0 k vs 10,6 k; Pedra max
  7 s). `kvrocks_set_mc50`: Adaptive **off** a n≥16; avg_group 1,0;
  **0,37×** — bypass, não grouping.
- RFC-0055 P0: 1c oficial não paga concurrent memtable; P1.1 parked
  *iff* mem-apply ≥15% do gap mc4/mc50. Esse iff **nunca foi
  reaberto** com o overwrite_mc4 ≥1× e o apply 0,47× na mesa.
- ycsb_a_mc4 p50 8,7 vs 3,2 µs (0,91× QPS) — misto; TLS write-through
  voltou (0180 P0.35); o p50 continua longe.

## Problems This Solves

- **Problem:** “ganhámos overwrite_mc4” esconde 1c 0,82× e apply 0,47×.
- **Problem:** 0055 P1 parked sem o número de 2026-09-07.
- **Problem:** skiplist no escuro (0055 avisou; 50c o teto é
  agendamento, hold 1,6 µs).

## Proposed Solution

Medir `WRITEPHASE` / `pipeline_gap` em apply_mc4 e 1c overwrite no
**mesmo** binário 0180. Se mem-apply ≥15% do gap vs Rocks default,
despark 0055 P1.1 (insert fora do write lock — **não** skiplist
concorrente no P0). Senão: ceiling nomeado na 0178 (1c fd/CPU;
apply serial; mc50 bypass). Adaptive 2–8 fica; n=50 não vira merge.

## Delivery slices (mandatory)

### P0 — o número na mesa

- [x] **P0.1** Este RFC — status: `done`
- [ ] **P0.2** `pipeline_gap` + `PEDRA_WRITE_PHASE_STATS=1` em
      `deps_apply_batch_mc4` e `deps_cache_overwrite` 1c vs Rocks
      `SYNC=0` (Darwin). Fração mem-apply / hold / wait. Finding.
      — status: `todo`
- [ ] **P0.3** Veredito escrito: despark 0055 P1.1 **ou** ceiling
      nomeado (1c / apply_mc4 / mc50) na 0178 — status: `todo`

### P1 — só se P0.3 despark

- [ ] **P1.1** Insert fora do write lock (0055 P1.1) **iff** P0.3
      obrigar ≥15% — status: `todo`
- [ ] **P1.2** apply_mc4 3-run Darwin vs Rocks default depois do
      P1.1 (ou skip se ceiling) — status: `todo`

### P2 — polish

- [ ] **P2.1** none yet — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este ficheiro | 2026-09-07 |
| P0.2 | p0 | pipeline_gap apply_mc4 + 1c | todo | — | 2026-09-07 |
| P0.3 | p0 | veredito despark ou ceiling | todo | — | 2026-09-07 |
| P1.1 | p1 | insert off-lock iff | todo | 0055 P1.1 | 2026-09-07 |
| P1.2 | p1 | apply_mc4 3-run | todo | — | 2026-09-07 |
| P2.1 | p2 | none yet | todo | — | 2026-09-07 |

## Acceptance Criteria

- **Tests:** finding com as fracções; se ceiling, uma frase na 0178
  com o número (não um skiplist).
- **Telemetry / Analytics:** `WRITEPHASE` dump; `pipeline_gap`
  stdout. Peer `"sync": false`.
- **Documentation:** este RFC; 0055 P1.1 aponta o iff; 0178 se
  ceiling.
- **Screenshots:** backend-only.

## Out of scope

- Skiplist concorrente no P0. G1 win. Fjall gate.
- overwrite_mc4 ≥1× (0180). Prefix 0,70× caixa (0178 P1.2).
- 50M/100M 4 GiB bounded-cache (0178 P2.2).
