# RFC-0054: fechar os buracos oficiais — `deps_raftlog` >1×, os outros ≥2×

**Status:** in-progress
**Updated:** 2026-08-23
**Parents:** [0041](0041-2x-rocks-default.md) (piso 2× vs Rocks default),
[0044](0044-async-class-5x-rocks.md) (coluna async 5×),
[AGENTS.md](../../AGENTS.md) (peer = `sync=false`)
**Evidence:** [`findings/2026-08-23-rearm8/`](../../findings/2026-08-23-rearm8/)

## Background

- Drop-in `rocksdb-compat::Options::sync` passa a **false** (este RFC, P0.0):
  mesma classe do peer oficial. Kernel `OpenOptions.sync` **continua true**.
  `wal_full_fsync=true` permanece: `set_sync(true)` no Darwin **é**
  `F_FULLFSYNC` (CMake-Rocks), não um knob extra na hora do Ok.
- Scoreboard oficial rearm8 (gate load<10 ×2, peer `sync:false`):
  **13/18 ≥ 2×**. Ganhamos em quase tudo. O que falta é curto e nomeado.

| forma | × Rocks | alvo deste RFC |
|---|---:|---|
| kvrocks_get / pipelined_set / ycsb_e | 5,58 / 5,20 / 5,44 | já fechado (0044) |
| A/D/SET/C/B/F / lock / mc50 | 4,6 … 2,12 | já ≥2 |
| **deps_apply_batch** | 2,08 med, **2/3** | P1.4: 3/3 ≥2 |
| deps_mvcc_latest | 1,79 | P1.1 ≥2 |
| kvrocks_blob_set | 1,70 | P1.2 ≥2 |
| deps_scan | 1,68 | P1.3 ≥2 |
| **deps_raftlog** | **0,59** | P0: **>1×** |

- `deps_raftlog` já não é cauda: stall APFS (F_PREALLOCATE) eliminado;
  máx 0,063–0,098 ms ≈ peer 0,047–0,068. Gap = caminho:
  13–15 µs/batch vs 7,6–8,0. Write core in-process **4,0 µs** p50;
  réplica bench-shape 8,3 µs; `probe2` (mesmo loop, DB maior) 8,0 µs.
  ~9 µs vivem no **harness do binário oficial**, não no encode.

## Problems This Solves

- **Problem:** única forma <1× no cartaz (`deps_raftlog` 0,59).
- **Problem:** três formas ≥1 e <2 (mvcc / blob / scan) e apply 2/3.
- **Problem:** o drop-in defaultava `sync=true` enquanto o peer e as pernas
  oficiais já eram async — duas classes no mesmo crate.

## Proposed Solution

1. Drop-in default = Rocks async. G1 vira `set_sync(true)`. Darwin forte
   só quando essa barreira dispara.
2. Instrumentar `PEDRA_WRITE_PHASE_STATS` **dentro** do loop `deps_raftlog`
  do `rocks-parity-bench` (não só o probe in-process) e cortar o que o
  número apontar até >1×.
3. Cada sub-piso: número antes de teoria (mesmo método do raftlog).

## Delivery slices (mandatory)

### P0 — drop-in Rocks-shaped + raftlog >1×

- [x] **P0.0** `Options::sync` default `false`; adversarial opt-in G1;
      `wal_full_fsync` default true — status: `done`
- [x] **P0.1** Example `raftlog_submit_probe` — status: `done`
      (release: write-core p50 **3,79 µs**; bench-shape p50 **5,50 µs**.
      Oficial 13–15 µs ainda é o processo do harness, não o encode)
- [ ] **P0.2** Cortar o gap nomeado por P0.1 até `deps_raftlog` **>1×** numa
      bateria quieta (1/3 já conta como evidência; fecha com 3/3) — status: `todo`

### P1 — os outros ≥2× + apply 3/3

- [ ] **P1.1** `deps_mvcc_latest` ≥2× (medir get-at / prefix cache / CF
      routing; não chutar memtable) — status: `todo`
- [ ] **P1.2** `kvrocks_blob_set` ≥2× (vlog spill vs inline; peer máx 11–14 ms
      é dele) — status: `todo`
- [ ] **P1.3** `deps_scan` ≥2× (L0 / count-cache / janela) — status: `todo`
- [ ] **P1.4** `deps_apply_batch` 3/3 ≥2 numa bateria quieta — status: `todo`

### P2 — polish

- [ ] **P2.1** OCC `WriteOptions.sync` honrado (hoje herda o DB; Surreal já
      seta false) — status: `todo`
- [ ] **P2.2** Gate `ROCKS_PARITY_RATIO_FLOOR=2.0` nas formas que P1 fechou
      — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.0 | p0 | drop-in sync=false | done | este change | 2026-08-23 |
| P0.1 | p0 | raftlog_submit_probe | done | example (core 3,79µs / shape 5,50µs) | 2026-08-23 |
| P0.2 | p0 | raftlog >1× quieto | todo | — | 2026-08-23 |
| P1.1 | p1 | mvcc_latest ≥2× | todo | — | 2026-08-23 |
| P1.2 | p1 | blob_set ≥2× | todo | — | 2026-08-23 |
| P1.3 | p1 | deps_scan ≥2× | todo | — | 2026-08-23 |
| P1.4 | p1 | apply 3/3 ≥2 | todo | — | 2026-08-23 |
| P2.1 | p2 | OCC WriteOptions.sync | todo | — | 2026-08-23 |
| P2.2 | p2 | floor 2.0 gated | todo | — | 2026-08-23 |

## Acceptance Criteria

- **Tests:** `dropin_default_sync_matches_rocks`; adversarial **com**
  `set_sync(true)` (Ok=durable); `wal_full_fsync_switches_barrier_class`
  inalterado (kernel).
- **Telemetry:** bateria quieta rearm-style; JSON `sync: false`;
  `peer_policy: rocks-default`. raftlog ratio min das 3 > 1.0; mvcc/blob/scan
  mediana ≥ 2.0; apply 3/3 ≥ 2.0.
- **Documentation:** este RFC, `docs/rocksdb-compat.md` (kernel vs drop-in),
  `docs/usage.md` row 1.
- **Screenshots:** none — backend-only.

## Out of scope

- 5× (RFC-0044).
- Concurrent memtable / multi-flush / pipelined write (RFC-0055 — medir
  primeiro; as formas oficiais são 1 cliente e flush-free).
- Mudar `OpenOptions.sync` do kernel (G1 permanece no motor Pedra).
- Peer `sync=true`.
