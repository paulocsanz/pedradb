# RFC-0054: fechar os buracos oficiais — `deps_raftlog` >1×, os outros ≥2×

**Status:** in-progress
**Updated:** 2026-08-23
**Parents:** [0041](0041-2x-rocks-default.md) (piso 2× vs Rocks default),
[0044](0044-async-class-5x-rocks.md) (coluna async 5×),
[AGENTS.md](../../AGENTS.md) (peer = `sync=false`)
**Evidence:** [`findings/2026-08-23-rearm8/`](../../findings/2026-08-23-rearm8/),
[`findings/2026-08-23-rearm9/`](../../findings/2026-08-23-rearm9/) (P0.2),
[`findings/2026-08-23-rearm10/`](../../findings/2026-08-23-rearm10/) (P1.1)

## Background

- Drop-in `rocksdb-compat::Options::sync` passa a **false** (este RFC, P0.0):
  mesma classe do peer oficial. Kernel `OpenOptions.sync` **continua true**.
  `wal_full_fsync=true` permanece: `set_sync(true)` no Darwin **é**
  `F_FULLFSYNC` (CMake-Rocks), não um knob extra na hora do Ok.
- Scoreboard rearm8 (baseline deste RFC): 13/18 ≥ 2×. rearm9 (com P0.0+P0.2):
  raftlog 0,59→**1,05** (alvo P0 cumprido); lock_prewrite 1,96 e apply 1,93
  caíram para a borda do piso (ruído de box + peer mais rápido), mvcc 1,41,
  blob 1,25 (peer acelerou), scan 1,80 — os quatro seguem como P1. Ganhamos
  em quase tudo; o que falta é curto e nomeado.

| forma | × Rocks | alvo deste RFC |
|---|---:|---|
| kvrocks_get / pipelined_set / ycsb_e | 5,58 / 5,20 / 5,44 | já fechado (0044) |
| A/D/SET/C/B/F / lock / mc50 | 4,6 … 2,12 | já ≥2 |
| **deps_apply_batch** | 1,93 (rearm9) | P1.4: 3/3 ≥2 |
| deps_mvcc_latest | ~~1,41~~ → **2,53 (3/3, rearm10)** | **P1.1 fechado** |
| kvrocks_blob_set | 1,25 (peer 31,6k→90,2k no rearm9) | P1.2 ≥2 |
| deps_scan | 1,89 (rearm10) | P1.3 ≥2 |
| **deps_raftlog** | ~~0,59~~ → **1,05 (3/3, rearm9)** | **P0.2 fechado** |

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
- [x] **P0.2** Cortar o gap nomeado por P0.1 até `deps_raftlog` **>1×** numa
      bateria quieta (1/3 já conta como evidência; fecha com 3/3) — status: `done`
      (rearm9 3/3: **1,036/1,054/1,043** vs rocks 131–134 k. Gap **não** era
      memtable — fases por forma mostram mem 2,6 µs igual nos dois runs; era
      `publish` 0,57→4,2 µs: `CountCache::record_dirty` alocando 2 `Box`/key/
      publish depois que um scan enche o cache. Fix: envelope conservador +
      watermark no `insert` (F204-safe). Per-CF tail vecs **refutado**.
      De quebra, F219: `tail_idx_range` cross-shard devolvia range vazio e
      `last_visible_under_prefix` perdia o tail — 4 testes vermelhos no
      `main` desde ae89515, corrigidos)

### P1 — os outros ≥2× + apply 3/3

- [x] **P1.1** `deps_mvcc_latest` ≥2× — status: `done`
      (rearm10 3/3: **2,18/2,53/2,98**. `last_visible_under_prefix`
      coletava TODAS as versões do usuário num Vec por chamada — usuário
      quente do zipf com dezenas de versões do apply. Agora max-of-maxes
      reverso por conjunto (map + shards do tail_idx), sem materializar;
      probe `last` 1649→400 ns. Era 1,41 no rearm9)
- [ ] **P1.2** `kvrocks_blob_set` ≥2× (vlog spill vs inline; peer máx 11–14 ms
      é dele) — status: `todo`
- [ ] **P1.3** `deps_scan` ≥2× — status: `partial`
      (step_user O(1) no cursor de count: 1,80→**1,89** (1,65/1,96/1,89,
      rearm10); TLS absolve ~45% das scans (874/2000 ao kernel). Falta
      cortar o caminho dos 874)
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
| P0.2 | p0 | raftlog >1× quieto | done | rearm9 3/3 (1,04 med); publish=count-cache tax; F219 | 2026-08-23 |
| P1.1 | p1 | mvcc_latest ≥2× | done | rearm10 3/3 (2,18/2,53/2,98) reverse-walk | 2026-08-23 |
| P1.2 | p1 | blob_set ≥2× | todo | 1,23/1,27/1,31 (rearm10, banda) | 2026-08-23 |
| P1.3 | p1 | deps_scan ≥2× | partial | 1,89 (step_user +10%) | 2026-08-23 |
| P1.4 | p1 | apply 3/3 ≥2 | todo | 1,82/1,93/1,84 (rearm10) | 2026-08-23 |
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
