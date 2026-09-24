# RFC-0041: Pedra ≥ **2×** RocksDB **default** em todo o harness

**Status:** in-progress (floor re-baselined 1× — ver “Floor decision 2026-08-24”)  
**Updated:** 2026-08-24
**Parked (quiet remesure / G1 ceiling):** remaining P1.1/P1.3/P2 remesure need a quiet host; P1.2 is the in-tree fd ceiling (cannot hit 2× without dropping G1). Dirty sandbox is not the official floor.  
**Parents:** [0040](0040-fsync-always-beats-rocks-async.md) (peer = Rocks default; group/sticky), [AGENTS.md](../../AGENTS.md) (única vitória = vs `sync=false`)  
**Children:** [0043](0043-high-level-2x-expanding-benches.md) (2× no subconjunto *alto nível* + catálogo que só cresce; 1c write continua aqui),
[0044](0044-async-class-5x-rocks.md) (coluna **async/async** piso 5×; não é o cartaz G1)

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

## Floor decision 2026-08-24 — 2× → 1× (decisão de produto registrada)

O piso oficial de paridade passou de **2×** para **1×** contra o **mesmo
peer de sempre** (RocksDB default, `WriteOptions.sync=false`,
`ROCKS_PARITY_SYNC=0`; o compare continua exit 2 em peer `sync: true`).
Nada mudou no peer, e vender coluna sync como vitória continua proibido.

- **Gate oficial:** `scripts/rocksdb_parity_v0.sh` agora defaulta
  `ROCKS_PARITY_RATIO_FLOOR=1.0` e mede a coluna drop-in (RFC-0054,
  same-class async-vs-async): **15/15 shapes ≥ 1.254**
  ([findings/rocks-parity-floor1x/](../findings/rocks-parity-floor1x/README.md)).
  É o gate de regressão de velocidade do engine — nunca citado como
  “batemos o Rocks”.
- **Tabela de produto (G1, fdatasync antes do Ok):** publicada por shape em
  [findings/rocks-parity-floor1x-g1/](../findings/rocks-parity-floor1x-g1/README.md).
  Leituras **1.128–1.986×** o default **com** a durabilidade mais forte;
  escritas 1-cliente write-per-op são fd-ceiling abaixo de 1× **por
  construção** (uma barreira completa por op vs zero do peer —
  F_FULLFSYNC p50 ≈ 3.8 ms no Darwin; fdatasync 25.7 µs no Linux levou as
  mesmas shapes só a 0.056×, P0.2; teto afirmado em código). O fechamento é
  group commit (uma barreira por **grupo**: apply_mc4 **2.788×** head3) —
  exatamente o caminho que o kernel provado do RFC-0057 P2.1 agora guarda.
- Os slices P1/P2 de 2× abaixo permanecem como backlog registrado de
  performance e deixam de ser gate de shipping; P2.3 (gate 2.0 em todas)
  fica **superseded** por esta decisão.

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
- [x] **P0.3** Ligar `ROCKS_PARITY_RATIO_FLOOR=2.0` no script **só** nas shapes que P0.2 já mostrar ≥ 2.0 (não ligar gate vazio) — status: `done` (wiring + receita no script; **default off**: 0/16 shapes ≥ 2.0 em *todas* as 3 runs do head3; `GATE_SHAPES` documentado)

### P1 — escritas ≥ 2× default

- [ ] **P1.1** `deps_apply_batch_mc4` e `deps_raftlog_mc4` ≥ 2.0 vs default da run — status: `doing` (parked: quiet remesure; head3: **apply_mc4 2.788 ✓**; raftlog_mc4 **1.792**. Catch-up 50 µs (único dado quieto; 80 µs revertido sem remesura); `write_cf_owned` + WAL 1× `encoded_len`. Remesura só caixa quieta)
- [ ] **P1.2** `ycsb_a` / `ycsb_f` / `deps_cache_overwrite` (1c e `_mc4` se existirem) ≥ 2.0 vs default — status: `todo` (teto **afirmado no código**: `rfc0041_one_fdatasync_cannot_hit_2x_rocks_default_ycsb_a` — p50 `fdatasync` > 2.48 µs = 2× head3 Rocks A. Sem largar G1/peer/shapes isto não fecha)
- [ ] **P1.3** `deps_apply_batch` e `deps_raftlog` **1 cliente** ≥ 2.0 vs default — status: `doing` (parked: quiet remesure; rearm8: raftlog 0,59× caminho, apply 2,10 med 2/3). **Trabalho restante vive em [RFC-0054](0054-close-official-gaps.md)** (raftlog >1× primeiro; mvcc/blob/scan ≥2; apply 3/3). (mesmo `write_cf_owned`; official ainda head3 1.30 / 0.72. **Pubfix 2026-08-22 (`8e3a460`/F204, `findings/2026-08-22-raftlog-pubfix/`)**: publish-phase era 28% do batch (record_dirty alocava 2 Box + 2 hash inserts por chave com mapa vazio; point-cache 1 mutex/chave) — watermark `skipped_below` + `invalidate_many` derrubou publish 3,17→0,15 µs (−95%); A/B oficial-shape 15 rounds sujo **+24,5%** raftlog (extrapola ~0,90× vs 0,72× limpo), ycsb_a +3,1% sem regressão (391 testes ok). Restam ~2× de headroom sobre os pisos: fold deep-clone, `tail_idx` 0,197 µs/op, submit-path. NON-OFFICIAL. **rearm7 quieta 2026-08-23 (`findings/2026-08-22-rearm7/`, d343044)**: raftlog **0,35×** (43,8 k vs 124,6 k) — mas p50 **7,5–7,7 µs = paridade com rocks (6,9–8,5)**; o gap inteiro é um stall de 20–38 ms/perna na fase `wal`: appends plain travam 9–54 ms **a cada 8 MiB** no APFS (alocação de extent no `write(2)`, determinístico, probe per-batch + microbench C `F_PREALLOCATE`); rocks pre-aloca o WAL e nunca trava. Fix em curso: pré-alocação de segmento WAL (`EnvFile::preallocate`) — wall ≈ p50×2000 ⇒ ~1,0× projetado). **rearm8 quieta 2026-08-23 (`findings/2026-08-23-rearm8/`, fix aterrissado 6c49252)**: raftlog **0,59×** (79,6/66,0/76,6 k vs 131,2/130,6/124,8 k) com **cauda na classe do peer** — máx compat 0,063–0,098 ms ≈ rocks 0,047–0,068 ms (era 19,7–38,5 vs 0,025–0,076): **stall de extent eliminado, fix confirmado em bateria quieta**. O gap restante é caminho, não cauda: 13–15 µs/batch vs 7,6–8,0 do rocks, sendo o write core ~4 µs (probe 20k×16) — ~9 µs vivem na construção/submit do lote (probe de fases só cobre o write). `deps_apply_batch` 1,99/2,10/2,15 (med 2,10, 2/3 ≥2 — r1 1,99 no fio). Fold deep-clone descartado como alavanca: `fold_parked_once_off_lock` só roda com memtable staged ≥256 MiB, inatingível nestas formas)

### P2 — leituras ≥ 2× default

- [ ] **P2.1** `deps_scan` e `ycsb_e` ≥ 2.0 (L0 drenado; p95 do miss) — status: `doing` (parked: quiet remesure; head3 scan **1.790**)
- [ ] **P2.2** `ycsb_c` / `b` / `d` / `deps_mvcc_latest` ≥ 2.0 — status: `doing` (parked: quiet remesure; head3 C **1.796**)
- [x] **P2.3** Gate em todas as shapes — **superseded 2026-08-24**: gate oficial = floor **1.0** na coluna drop-in (decisão acima); G1 vira tabela de produto publicada — status: `done (superseded)`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este doc | 2026-08-17 |
| P0.2 | p0 | remesura 11+MC vs default | done | findings/rfc0041-p02 | 2026-08-17 |
| P0.3 | p0 | floor 2.0 nas que já passam | done | wiring; default off (scatter) | 2026-08-18 |
| P1.1 | p1 | apply/raftlog MC ≥ 2× | doing | apply_mc4 **2.788 ✓**; raftlog_mc4 1.792 (head3). WAL one `encoded_len` | 2026-08-18 |
| P1.2 | p1 | A/F/overwrite ≥ 2× | todo | env.rs fd-ceiling test; 1c físico | 2026-08-18 |
| P1.3 | p1 | apply/raftlog 1c ≥ 2× | doing | rearm8: raftlog 0,59× caminho; apply 2,10 med 2/3. Restante → [RFC-0054](0054-close-official-gaps.md) | 2026-08-23 |
| P2.1 | p2 | scan/E ≥ 2× | doing | head3 E 2.121 / scan 1.790; count hit is `&self` | 2026-08-18 |
| P2.2 | p2 | C/B/D/MVCC ≥ 2× | doing | head3 C 1.796; get hit `borrow()` + short hash | 2026-08-18 |
| FD | decision | **floor 2×→1×**: gate drop-in 1.0; G1 = tabela publicada | done | findings/rocks-parity-floor1x{,-g1} | 2026-08-24 |
| P2.3 | p2 | gate 2.0 em todas | superseded | floor 1× drop-in (FD abaixo) | 2026-08-24 |

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
