# RFC-0039: apply / raftlog / scan ≥5× Rocks **sync** (async continua a coluna honesta)

**Status:** draft  
**Updated:** 2026-08-17  
**Parents:** [0037](0037-apply-off-put-and-2x-pedra.md) (apply off-put, host worker, gate 0.5), [0036](0036-tikv-rocks-2x-fdatasync.md) (WAL `fdatasync` = classe TiKV), [0031](0031-rocks-parity-10x-budget.md) (G1–G8)

## Background

- Pedido permanente do dono: **sempre** comparar Pedra com Rocks **async** (`WriteOptions.sync=false`). Essa coluna é o relatório default. Este RFC **não** a substitui e **não** promete 5× vs async (async não paga `fdatasync` por write; ~7–10× o sync nas escritas — outra classe).
- Pedido deste RFC: nos três shapes que ainda perdem ou empatam no p50/p95 contra o peer **sync** (`WriteOptions.sync=true` / `fdatasync`, `ROCKS_PARITY_FULL_SYNC=0`), Pedra tem de ser **no mínimo 5×** esse Rocks sync. Os três: `deps_apply_batch`, `deps_raftlog`, `deps_scan`.
- Baseline quieto (run5, [findings/tikv-ycsb-concurrent-compat](../findings/tikv-ycsb-concurrent-compat/), compat já em `ConcurrentDb`):

| shape | Pedra qps | p50 | p95 | Rocks **sync** qps | p50 | p95 | hoje | **alvo 5×** (qps) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| deps_apply_batch | 1 776 | 350 µs | 1.66 ms | 2 790 | 224 µs | 428 µs | 0.64× | **13 950** |
| deps_raftlog | 3 036 | 164 µs | 1.22 ms | 1 688 | 115 µs | 3.08 ms | 1.80× (cauda Rocks) | **8 440** |
| deps_scan | 154 707 | 0.3 µs | **22 µs** | 252 255 | 3.7 µs | **5.0 µs** | 0.61× | **1 261 000** |

- O que o timer mede (agenda fixa, 4096/2000 zipfian 1 KB, 1 cliente):
  - **apply:** 2 `write()` por op (prewrite 64 ops + commit 64 ops). Dois `fdatasync`. ~32 KB de valor no prewrite.
  - **raftlog:** 1 `write()` de 16×1 KB + `fdatasync`; get a cada 8.
  - **scan:** `count_cf` de 25 chaves no CF `write`. p50 é `count_cache`; p95 é o merge real (run5: 7 SST, 2.7 sondados/scan).

### Piso físico (G8, não é desculpa — é conta)

- Apply: **2** `fdatasync` por op cronometrada. Se `fdatasync` ≈ 30 µs, piso ≈ 60 µs → teto ≈ 16.7 k qps. Alvo 13.95 k cabe **só se** o CPU cair para ~10 µs e **não** houver flush/compact no timed path. Se a caixa medir `fdatasync` ≥ 40 µs, 5× apply **não fecha** sem saltar um sync (proibido, G1) ou fundir os dois `write()` (muda o schedule / não é o Rocks). P0.2 mede o syscall nesta caixa e grava o número. Sem esse número, 5× apply é hipótese.
- Raftlog: **1** `fdatasync`. Rocks p50 115 µs / média ~590 µs (max 218 ms). 8.4 k qps = 119 µs média — **dentro** de um fd + CPU curto, se matarmos a nossa cauda.
- Scan: 5× = 0.79 µs média. Já ganhamos o p50 (cache). O miss a 22 µs tem de ir a ~1 µs, **ou** o hit-rate tem de dominar o average. Não é o tipo do lock.

## Problems This Solves

- **Problem:** apply/raftlog pagam 2–3 cópias do payload (encode CF → `Bytes`, `WriteRecord::encode` → `Vec` do WAL, `ChangeEntry` mesmo com `changelog_interval=0`) **além** do `fdatasync`. Isso é a fatia que ainda cabe antes do piso.
- **Problem:** scan qps perde no p95 (abrir 3–7 cursores SST com `Box` + `Bytes` dos bounds + L0=4), não no p50.
- **Problem:** relatórios misturam Rocks sync e async. 5× vs sync sem a coluna async ao lado é o erro que o dono já mandou parar.

## Proposed Solution

1. **Toda remesura deste RFC publica três colunas:** Pedra · Rocks sync · Rocks async. Gate **só** nos três shapes, **só** vs sync, floor **5.0**. Async é relatório permanente, nunca denominador do 5×.
2. **Apply / raftlog:** uma cópia do payload até o WAL; não materializar `ChangeEntry` quando `changelog_interval=0`; compact/flush fora do `write()` timed (worker já existe; auto-flush não pode meter merge no Ok). Sem fundir os dois `write()` do apply. Sem skip de sync.
3. **Scan:** cursor SST sem `Box`/`Bytes` dos bounds; menos L0 visíveis no shape (worker drena até < trigger **antes** de o apply deixar 4 L0 no colo do scan); count sem `user_key.clone()` por passo.
4. **G1–G8 intactos.** Adversarial sem editar asserção. Sem thread no core.

## Garantias invariáveis

| # | Garantia | Este RFC |
|---|---|---|
| G1 | WAL `fdatasync` antes de **cada** `write()` Ok | intocada — apply continua com **dois** syncs |
| G2 | visibilidade = lookup / range_at | count/merge emitem o mesmo conjunto |
| G4 | adversarial compat | re-verde **sem** editar asserção |
| G5 | fence se sync falha | intocada |
| G6 | sem thread no core | worker fica no host (já shipped) |
| G8 | números honestos | 5× só vs sync da **mesma** run; async sempre na tabela; se P0.2 provar piso < 5× apply, o slice fica `todo` e o finding diz o piso — **não** se relabela o alvo |

## Delivery slices (mandatory)

### P0 — must ship first (útil sozinho: piso medido + uma cópia a menos)

- [x] **P0.1** RFC + Status vivo (este doc) — status: `done`
- [ ] **P0.2** Medir nesta caixa: `fdatasync` p50 isolado; split apply/raftlog (encode CF / `WriteRecord` / mem / fd / flush); split scan (cache hit vs miss, nº SST, setup vs merge). Publicar Pedra · sync · **async**. Escrever se 5× apply cabe no 2×fd — status: `todo`
- [ ] **P0.3** WAL encode sem segunda cópia do payload + skip `ChangeEntry` quando `changelog_interval=0`. Teste: reopen ainda reconstrói o feed. Remesura apply/raftlog p50 — status: `todo`

### P1 — apply e raftlog ≥ 5× sync

- [ ] **P1.1** `deps_raftlog` ≥ **5.0** vs Rocks sync da run (matar cauda de flush no timed path; 1 fd + CPU curto) — status: `todo`
- [ ] **P1.2** `deps_apply_batch` ≥ **5.0** vs Rocks sync da run. Se P0.2 mostrou 2×fd > Rocks_avg/5, finding com o piso e o slice permanece `todo` até o dono rever o número — status: `todo`

### P2 — scan ≥ 5× sync

- [ ] **P2.1** Cursor SST de count sem `Box<dyn>` e sem copiar bounds para `Bytes`; `count_visible` sem `user_key.clone()` — status: `todo`
- [ ] **P2.2** Apply não deixa L0 ≥ trigger para o scan: worker drena até `< L0_COMPACTION_TRIGGER` (já é o contrato; fechar a corrida) — status: `todo`
- [ ] **P2.3** `deps_scan` ≥ **5.0** vs Rocks sync da run (p95 do miss perto do Rocks 5 µs, ou hit-rate que puxe a média a ≤ 0.79 µs) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este doc | 2026-08-17 |
| P0.2 | p0 | piso fd + split + coluna async | todo | — | 2026-08-17 |
| P0.3 | p0 | uma cópia WAL + skip ChangeEntry | todo | — | 2026-08-17 |
| P1.1 | p1 | raftlog ≥ 5× sync | todo | — | 2026-08-17 |
| P1.2 | p1 | apply ≥ 5× sync | todo | — | 2026-08-17 |
| P2.1 | p2 | cursor count sem Box | todo | — | 2026-08-17 |
| P2.2 | p2 | L0 drenado antes do scan | todo | — | 2026-08-17 |
| P2.3 | p2 | scan ≥ 5× sync | todo | — | 2026-08-17 |

## Acceptance Criteria

- **Tests:** `cargo test -p rocksdb-compat` adversarial **sem** editar asserção; reopen com `changelog_interval=0` ainda vê o feed (P0.3); count/scan = mesmo conjunto que hoje (P2.1).
- **Telemetry:** `scripts/tikv_ycsb_parity_v0.sh` FULL_SYNC=0 **e** uma run Rocks `ROCKS_PARITY_SYNC=0` (async) na mesma máquina. Finding com p50/p95/p99 e as três colunas. Gate: `compat_qps / rocks_sync_qps ≥ 5.0` em apply, raftlog e scan (medianas de ≥3 runs quietas, não um spike).
- **Documentation:** este RFC + finding da remesura. backend-only.
- **Screenshots:** none — backend-only.

## Out of scope

- 5× vs Rocks **async**. Async é coluna obrigatória, não o gate.
- 5× nos outros 8 shapes (A–F, mvcc, overwrite).
- Fundir prewrite+commit num `fdatasync` (não é o schedule do Rocks).
- Skip de sync no Ok (G1).
- Thread no `pedradb-core` (G6).
- Relabel de L0→L1 sem rewrite (já medido no 0037: parte o scan).
