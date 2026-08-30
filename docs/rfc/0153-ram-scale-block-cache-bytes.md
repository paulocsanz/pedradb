# RFC: 0153 — RAM de verdade: block cache em bytes

**Status:** in-progress (P0 done; P1.1 → RFC-0154; P2 open)
**Updated:** 2026-08-29
**Parents:** [0002](0002-internal-key-memtable.md) (arena recusada até o write path pedir),
[0012](0012-research-decisions.md) (BTreeMap até CPU no memtable),
[0062](0062-launch-readiness-remaining-gaps.md) (knobs inertes nomeados)

**Evidence:** apply 2× no metal é memtable sem flush
([`findings/2026-08-29-linux-p149-five/`](../../findings/2026-08-29-linux-p149-five/)).
Isso não é curva de TB + RAM.

## Background

- Block cache do kernel é **8192 entradas**, não bytes. `Cache::new_lru_cache(1<<30)` e
  `optimize_for_point_lookup(n)` são no-op. `rocksdb.block-cache-usage` devolve
  hits+misses, não occupancy.
- Memtable: tail `Vec` + `tail_idx` vivo (RFC-0154). Harness pina 256 MiB para
  **não** flushar; isso não é curva de TB.
- RFC-0002 P1.2 recusou arena/skiplist até o write path pedir. O apply 2× pediu
  CPU no memtable, mas arena não cabe num P0. RAM extra no host, hoje, não
  acelera SST.

## Problems This Solves

- **Problem:** Host com RAM grande chama `set_block_cache` / `optimize_for_point_lookup`
  e o working set SST não cresce.
- **Problem:** `BLOCK_CACHE_USAGE` mente (hits+misses).
- **Problem (P1+):** memtable sem arena não é o padrão de tabela grande.
  Índice preguiçoso fechou em [RFC-0154](0154-memtable-index-is-an-invariant.md).

## Proposed Solution

- Block cache com **orçamento em bytes** (payload key+value+trailer). Default
  open sem knob permanece 8192 entradas (não regressar YCSB C).
- `Cache::new_lru_cache(n)` e `optimize_for_point_lookup(mb)` dimensionam o
  budget. Evicção LRU por tick (já O(1) hit).
- Property `rocksdb.block-cache-usage` = bytes ocupados.

## Delivery slices (mandatory)

### P0 — must ship first (RAM extra mexe no SST)

- [x] **P0.1** `BlockCache` budget em bytes + `used_bytes`; default 8192
      entradas inalterado — status: `done`
- [x] **P0.2** `set_block_cache` / `optimize_for_point_lookup` /
      `BlockBasedOptions::set_block_cache` wired; property USAGE = bytes —
      status: `done`

### P1 — next wave (memtable escala sem desfazer apply 2×)

- [x] **P1.1** Índice ao vivo incremental no `insert_many` (sem rebuild O(n)
      no primeiro get); `write_log` fundido — status: `done`
      ([RFC-0154](0154-memtable-index-is-an-invariant.md) P0+P1)

### P2 — later

- [ ] **P2.1** Arena / skip-list se P1.1 ainda for CPU-bound no write buffer
      real (64 MiB), reabrindo RFC-0002 P1.2 — status: `todo`
- [ ] **P2.2** Bateria oficial com flush na janela (64 MiB, não 256) —
      status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | block cache byte budget | done | cache.rs with_budget_bytes | 2026-08-29 |
| P0.2 | p0 | knobs + BLOCK_CACHE_USAGE | done | Options + property USAGE | 2026-08-29 |
| P1.1 | p1 | live incremental idx | done | RFC-0154 P0+P1 | 2026-08-29 |
| P2.1 | p2 | arena iff still bound | todo | — | 2026-08-29 |
| P2.2 | p2 | flush-in-window battery | todo | — | 2026-08-29 |

## Acceptance Criteria

- **Tests:** `block_cache_byte_budget_evicts_cold`; compat
  `optimize_for_point_lookup_sizes_cache`; `block_cache_usage_is_bytes`.
- **Telemetry / Analytics:** `rocksdb.block-cache-usage` = occupancy.
- **Documentation:** este RFC; knobs `Wired`; open-items uma linha.
- **Screenshots:** backend-only.

## Out of scope

- Titan / per-CF block cache. RFC-0149 P2.1 CHV remedir. Arena neste
  slice (P2.1). Lazy idx foi RFC-0154, não este P0.
