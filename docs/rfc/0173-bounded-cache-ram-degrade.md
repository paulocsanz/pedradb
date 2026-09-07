# RFC-0173 — Bounded-cache: degradar para lentidão, não para Err/OOM

**Status:** done
**Updated:** 2026-09-06
**Parents:** [0168](0168-vitoria-por-celula-toda-escala.md) (WARM default + cap 3 GiB),
[0162](0162-hydrate-scale-rate-decay.md) (DONTNEED por chunk no bulk),
[0153](0153-ram-scale-block-cache-bytes.md) (block cache em bytes)

## Background

- Default WARM (RFC-0168 P1.1) preenche o page cache do kernel no mesmo fd
  do `get`. 10M (~2,4 GiB) cabe; 25M/100M passam do cap e **só pulam o
  stream**. As páginas que o hydrate/flush deixou no kernel continuam lá —
  o cgroup Linux cobra file cache + RSS e o 4 GiB box SIGKILL se o store
  inteiro ficar pinado.
- Bulk já dá `posix_fadvise(DONTNEED)` no chunk (RFC-0162). Memtable flush
  e compact **não**. O over-cap era um no-op silencioso.
- Writes recusar com `Err(RamBudget)` foi recusado: o produto é lentidão,
  não falha de put. Índice+bloom deixam de ser always-resident no
  bounded-cache (P2.1+P2.3): first-keys paginam do tail; handles em
  RAM são 1 sample / 32 blocos.

## Problems This Solves

- **Problem:** store > RAM cap ainda deixa o working set no page cache.
- **Problem:** operador não vê que o ponto ficou disk-bound nem quanto
  RAM realocar.

## Proposed Solution

Três modos, escolhidos pelo tamanho do SST vivo vs cap
(`PEDRA_SETTLE_WARM_MAX_BYTES`, default `max(3 GiB, 3/4 RAM)`):

- **hot** — store cabe: WARM o conjunto (get_hit RAM-speed).
- **bounded-cache** (intermediário) — store não cabe: skip WARM +
  `POSIX_FADV_DONTNEED` em todo SST vivo. Point path = block cache
  (default 256 MiB no harness) + pread 4 KiB. Writes Ok. Darwin:
  DONTNEED é no-op (cgroup charging é Linux).
- **cold** — mesmo I/O se o kernel ignorar o hint.

Aviso one-shot (`tracing::warn` + `RAMPRESSURE` stderr) + métrica
(`DbStats` / `pedra.ram-pressure`). Não recusar write.

Aquecer só os primeiros N GiB de um store aleatório de 23 GiB é ~13%
hit — não vale. DONTNEED-all + LRU do block cache é o intermediário.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)

- [x] **P0.1** Over-cap skip WARM **e** DONTNEED nos SSTs vivos —
      status: `done`
- [x] **P0.2** Warn one-shot + métrica `pedra.ram-pressure` /
      `ram-warm-skipped` / `ram-ceiling-bytes` /
      `engine-resident-bytes` — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)

- [x] **P1.1** Harness scale imprime `mode=bounded-cache` quando a
      property é 1 — status: `done`

### P2 — later / polish

- [x] **P2.1** Índice+bloom evictable (lazy-index LRU) — first-keys
      saem do heap (pread do tail on-disk + LRU 64 MiB); bloom cai
      se metadata > 512 MiB. Writes Ok —
      status: `done`
- [x] **P2.3** Índice de dois níveis: RAM guarda 1 sample / 32 blocos,
      o run é um pread do tail. 1B keys ≈ 50M blocos → ~1,6M samples
      (~20 MiB) em vez de 1,4 GiB de SoA — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Over-cap DONTNEED (bounded-cache) | done | `enter_bounded_cache_mode`; `rfc0168_settle_warm_cap_skips_over_budget` | 2026-09-06 |
| P0.2 | p0 | RAMPRESSURE warn + properties | done | `DbStats` + `pedra.ram-*` | 2026-09-06 |
| P1.1 | p1 | Scale `mode=bounded-cache` | done | `PedraScale::settle` | 2026-09-06 |
| P2.1 | p2 | Lazy-index LRU | done | `page_index` file-backed; `page_indexes_for_ram`; `rfc0173_paged_index_loads_keys_from_file` + `rfc0173_bounded_cache_pages_index_and_gets` | 2026-09-06 |
| P2.3 | p2 | Índice dois níveis (fanout 32) | done | `INDEX_HANDLE_FANOUT`; `rfc0173_two_level_handles_are_sublinear` | 2026-09-06 |

## Acceptance Criteria

- **Tests:** `rfc0168_settle_warm_cap_skips_over_budget` — over-cap não
  streama, `ram_pressure==1`, `ram_warm_skipped>=1`, pelo menos um
  `AdviseKind::DontNeed` whole-file; under-cap ainda streama.
- **Telemetry / Analytics:** `pedra.ram-pressure` (0/1),
  `pedra.ram-warm-skipped`, `pedra.ram-ceiling-bytes`,
  `pedra.engine-resident-bytes`.
- **Documentation:** este RFC.
- **Screenshots:** backend-only.
- **Product:** put/flush/compact **não** devolvem Err por RAM. Degradação
  é lentidão. P2.1 pagina first-keys; P2.3 amostra handles 1/32. Bloom
  pode cair se metadata > 512 MiB.

## Out of scope

- Recusar write (`CoreError::RamBudget`) — revertido de propósito.
- Partial-warm dos primeiros N GiB.
- Mudar o tamanho default do block cache.
- Claim de get_hit@100M RAM-speed num box 4 GiB.
