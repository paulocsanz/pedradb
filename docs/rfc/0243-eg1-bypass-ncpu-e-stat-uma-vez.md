# RFC: Quatro escritores no bypass, e o WAL mmap não faz stat por frame

**Status:** in-progress
**Updated:** 2026-09-22

## Background
- O peer oficial é RocksDB default `sync=false`. A coluna medida é a async (write antes do Ok, sem `fdatasync`).
- RFC-0242 trocou o `pwrite` por frame do WAL `*.log` por `MAP_SHARED`. Na caixa 4 vCPU (`:p244`, canary r1 175772 e r2 225161, ambos ≥165000, `sync: false`):
  - A4 `deps_cache_overwrite_mc4` pedra 38972 / rocks 219603 = **0,1775** (antes 0,1384). `avg_group=2,04`. A semente de 25M, 1 escritor, fez 85400 puts/s — quatro escritores juntos foram mais lentos que um.
  - A5 `ycsb_f_mc4` 136474 / 230045 = **0,593** (antes 0,5425).
  - C2 `fjall_seq_1m` pedra 241788 vs fjall 396321 no mesmo host (antes 266730 vs 429138). Ainda abaixo. O 1c está em ~4,1 µs/op.
- O mmap chama `metadata()` em todo frame. Isso é um `stat` no caminho de 4,1 µs.
- `rmw_sched` default ON manda `writers == ncpu` para o grupo. O selo não junta os quatro; o hop do líder fica no caminho.

## Problems This Solves
- **Problem:** A4/A5 pagam o grupo de ~2 mesmo sem barreira de disco para amortizar, e ficam abaixo do 1 escritor.
- **Problem:** C2 paga `stat` por frame num orçamento de poucos microssegundos.

## Proposed Solution
- Default de `rmw_sched` volta a OFF. `writers == ncpu` usam o commit direto. `PEDRA_RMW_SCHED=1` restaura o grupo. Quem passa de `ncpu` continua no merge da política 0201.
- O handle do arquivo guarda `(dev, ino, is_wal)` depois do primeiro `metadata()`. Frames seguintes não fazem stat. Rotação abre outro handle.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** WAL mmap faz `metadata()` uma vez por arquivo aberto, não por frame — status: `done`
- [x] **P0.2** Quatro escritores em `ncpu` committam no bypass; o grupo fica opt-in — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [ ] **P1.1** Re-meter A4, A5 e C2 na caixa, canary ≥165000, peer `sync: false` — status: `todo`

### P2 — later / polish
- [ ] **P2.1** Se C2 ainda ficar abaixo do fjall depois do stat cache, o próximo dono é o CPU do put 1c (encode + mem), não outro sink — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | stat uma vez por WAL aberto | done | este change | 2026-09-22 |
| P0.2 | p0 | bypass default em writers == ncpu | done | este change | 2026-09-22 |
| P1.1 | p1 | re-meter Linux A4/A5/C2 | todo | — | 2026-09-22 |
| P2.1 | p2 | CPU do put 1c se C2 seguir impago | todo | — | 2026-09-22 |

## Acceptance Criteria
- **Tests:** `rfc0242_small_puts_skip_per_op_pwrite_and_reopen` (stat ≤ 8 em 2000 puts, cópias mmap ≥ 1500, reopen); `rfc0211_env_axis_rmw_sched_forms_groups` (default `queued == 0`, `PEDRA_RMW_SCHED=1` forma grupo); `concurrent_async_one_op_overwrite_is_visible` (bypass e as chaves continuam visíveis).
- **Telemetry / Analytics:** none — o contador `WAL_MMAP_STATS` existe para o teste, não é produto.
- **Documentation:** este RFC e a linha da escada em `docs/TRAJETORIA.md`.
- **Screenshots:** backend-only.

## Out of scope
- Não é win contra `sync=true`.
- Não declara A4/A5/C2 pagos antes do re-meter.
- Não mexe no selo `seal_async_first_drain` (o grupo opt-in continua selando cedo).
