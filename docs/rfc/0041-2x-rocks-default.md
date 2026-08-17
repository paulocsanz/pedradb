# RFC-0041: Pedra ≥ **2×** RocksDB **default** em todo o harness

**Status:** in-progress  
**Updated:** 2026-08-17  
**Parents:** [0040](0040-fsync-always-beats-rocks-async.md) (peer = Rocks default; group/sticky), [AGENTS.md](../../AGENTS.md) (única vitória = vs `sync=false`)

## Background

- Peer oficial: RocksDB **default** (`WriteOptions.sync=false`). Pedra no Ok faz `fdatasync`. O produto é os dois: mais durável **e** mais rápido que o Rocks que corre por aí. `rocks-parity-compare` recusa peer `sync=true` (`63767d0`).
- Pedido: **sempre** pelo menos **2×** esse Rocks — `compat_qps / rocks_default_qps ≥ 2.0` em **cada** shape do harness, mediana de ≥3 runs. Não é 2× o Rocks sync. Não é “empatar.”
- Harness: `scripts/tikv_ycsb_parity_v0.sh` (já default `ROCKS_PARITY_SYNC=0`) + MC quando `ROCKS_PARITY_CLIENTS=4`.
- Shapes: YCSB A–F, apply, MVCC, scan, raftlog, overwrite (1 cliente) **e** A/F/overwrite/apply/raftlog `_mc4`.
- Oficial vs default (mediana 3 runs, [rfc0041-p02](../../findings/rfc0041-p02/README.md); `fdatasync` isolado p50 **25.7 µs** ≈ 38.9 k qps/fd):

| shape | vs default | 2× pede |
|---|---:|---|
| ycsb_e | 0.91 | ~2.2× o qps atual |
| apply MC4 | 0.89 | ~2.3× |
| mvcc | 0.79 | ~2.5× |
| ycsb_c | 0.78 (p50 já ganha) | ~2.6× |
| raftlog MC4 | 0.53 | ~3.8× |
| scan qps | 0.50 (p50 já ganha) | ~4.0× |
| apply 1c | 0.39 | ~5.1× |
| YCSB A/F 1c | 0.056 / 0.073 | teto 1/fd ≪ 2× Rocks |

- 2× num put 1 cliente com um `fdatasync` por Ok **não fecha** nesta caixa: Rocks A ~398 k, 2× = 797 k, `1/t_fd` ≈ 39 k. P1.2/P1.3 1c **ficam `todo`** — não se muda o alvo nem se troca o peer. apply_mc4 **não** é o fd (p50 852 µs, fd 26 µs).

## Problems This Solves

- **Problem:** o alvo 0040 era “passar” o default. 2× é o piso de produto.
- **Problem:** sem uma remesura **só** vs default em **todas** as 11+MC, o gap é mistura de runs velhas.
- **Problem:** apply/raftlog 1c e scan qps ainda < 1.0 vs default; A/F 1c sequer têm número oficial neste peer.

## Proposed Solution

1. **Gate único:** `ROCKS_PARITY_RATIO_FLOOR=2.0` contra peer `sync=false`. Compare já recusa `sync=true`.
2. **P0 — mapa.** Remesura completa (11 + MC4) vs default; tabela gap; `fdatasync` p50 isolado na mesma caixa.
3. **P1 — escritas ≥ 2× default.** Group/pipeline (fd/N), CPU do WAL, compact fora do Ok. G1 intacto.
4. **P2 — leituras ≥ 2× default.** Scan p95 / L0; C/B/D/E/MVCC o que P0.2 deixar < 2.0.

## Garantias invariáveis

| # | Garantia | Este RFC |
|---|---|---|
| G1 | `fdatasync` antes do Ok (ou do Ok do grupo) | intocada |
| G2 | visibilidade = lookup / range_at | intocada |
| G4 | adversarial sem editar asserção | re-verde |
| G6 | sem thread no core | pipeline só no host |
| G8 | único denominador = Rocks default; mediana ≥3; sem vender sync como vitória | **o RFC** |

## Delivery slices (mandatory)

### P0 — must ship first (número oficial em todas as shapes)

- [x] **P0.1** RFC + Status vivo (este doc) — status: `done`
- [x] **P0.2** Remesura 11 + `_mc4` vs `ROCKS_PARITY_SYNC=0` apenas; p50/p95/qps; `fdatasync` isolado; finding `findings/rfc0041-p02/` — status: `done` (**0/16 ≥ 2.0**; apply_mc4 0.89; A 1c 0.056 ≪ 1/fd)
- [ ] **P0.3** Ligar `ROCKS_PARITY_RATIO_FLOOR=2.0` no script **só** nas shapes que P0.2 já mostrar ≥ 2.0 (não ligar gate vazio) — status: `todo` (conjunto vazio; não ligar)

### P1 — escritas ≥ 2× default

- [ ] **P1.1** `deps_apply_batch_mc4` e `deps_raftlog_mc4` ≥ 2.0 vs default da run — status: `doing` (MANIFEST `fsync` fora do write lock + skip catch-up em batch ≥16 ops; **ainda < 2.0** — [p11](../../findings/rfc0041-p11/README.md))
- [ ] **P1.2** `ycsb_a` / `ycsb_f` / `deps_cache_overwrite` (1c e `_mc4` se existirem) ≥ 2.0 vs default — status: `todo`
- [ ] **P1.3** `deps_apply_batch` e `deps_raftlog` **1 cliente** ≥ 2.0 vs default — status: `todo`

### P2 — leituras ≥ 2× default

- [ ] **P2.1** `deps_scan` e `ycsb_e` ≥ 2.0 (L0 drenado; p95 do miss) — status: `todo`
- [ ] **P2.2** `ycsb_c` / `b` / `d` / `deps_mvcc_latest` ≥ 2.0 — status: `todo`
- [ ] **P2.3** Gate 2.0 em **todas** as shapes do harness; script default `SYNC=0` `FLOOR=2.0` — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este doc | 2026-08-17 |
| P0.2 | p0 | remesura 11+MC vs default | done | findings/rfc0041-p02 | 2026-08-17 |
| P0.3 | p0 | floor 2.0 nas que já passam | todo | conjunto vazio | 2026-08-17 |
| P1.1 | p1 | apply/raftlog MC ≥ 2× | doing | MANIFEST off-lock; still <2.0 | 2026-08-17 |
| P1.2 | p1 | A/F/overwrite ≥ 2× | todo | — | 2026-08-17 |
| P1.3 | p1 | apply/raftlog 1c ≥ 2× | todo | — | 2026-08-17 |
| P2.1 | p2 | scan/E ≥ 2× | todo | — | 2026-08-17 |
| P2.2 | p2 | C/B/D/MVCC ≥ 2× | todo | — | 2026-08-17 |
| P2.3 | p2 | gate 2.0 em todas | todo | — | 2026-08-17 |

## Acceptance Criteria

- **Tests:** adversarial compat **sem** editar asserção; compare **recusa** peer `sync=true`.
- **Telemetry:** `tikv_ycsb_parity_v0.sh` com `ROCKS_PARITY_SYNC=0`; MC4 no mesmo out; mediana ≥3; `min_ratio ≥ 2.0` no conjunto gated.
- **Documentation:** este RFC + `AGENTS.md` (peer). backend-only.
- **Screenshots:** none — backend-only.

## Out of scope

- 2× vs Rocks `sync=true` (não é o peer).
- Tirar o `fdatasync` do Ok.
- Thread no `pedradb-core`.
- Relabel L0→L1 sem rewrite.
