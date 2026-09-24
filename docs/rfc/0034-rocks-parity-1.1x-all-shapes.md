# RFC-0034: 1.1× ceiling vs Rocks on every tabulated shape

**Status:** draft  
**Updated:** 2026-08-24
**Parked (quiet remesure):** remaining P-slices need a quiet 3× host; dirty sandbox numbers are not the official floor (AGENTS.md peer `sync=false`).  
**Parents:** [0033](0033-mvcc-scan-2x.md), [0032](0032-tikv-mix-2x-budget.md) (teto 2×), [0031](0031-rocks-parity-10x-budget.md) (G1–G8 + classe de sync)

## Background

- RFC-0032 trava `Pedra / Rocks_F_FULLFSYNC ≥ 0.5` (Pedra no máximo **2×** mais lento) na tabela TiKV-mix. RFC-0033 fechou o read-path absurdo (MVCC 3.2→1 190 qps; scan 13→2 778). O teto 2× ainda deixa MVCC e scan fora, e várias escritas mistas abaixo de 1.1×.
- Pedido: **todos** os shapes desta tabela no máximo **1.1× menos performáticos** que Rocks. Isso é `Pedra / Rocks ≥ 1/1.1 ≈ 0.909` (arredondamos o gate a **0.91**). Pedra pode ser mais rápido; não pode ficar 10%+ atrás.
- Duas colunas Rocks **não são o mesmo contrato**. Neste Mac: `fdatasync` p50 ~50 µs; `F_FULLFSYNC` (`File::sync_all`) p50 ~4.8 ms (~100×). `librocksdb-sys` desta build **não** define `HAVE_FULLFSYNC` — `WriteOptions.sync=true` é fdatasync. Pedra no Ok faz `sync_all` = `F_FULLFSYNC`.
- **1.1× de escrita vs fdatasync, mantendo G1, é fisicamente impossível** (um `F_FULLFSYNC` ≈ 4.8 ms vs ~50 µs). O peer oficial continua `ROCKS_PARITY_FULL_SYNC=1`. A coluna fdatasync fica report-only nas escritas. Nas leituras (C, MVCC, scan, e a parte read de B/D/E) get/iter **não** pagam sync — 1.1× vs fdatasync e vs FF é o mesmo trabalho.

Knobs iguais aos da tabela 0032: `scripts/tikv_ycsb_parity_v0.sh`, zipfian, 1 KB, 4096/2000, 1 thread. Floor = **0.91 × peer FF da mesma run**, não um qps cravado para sempre.

## Problems This Solves

- **Problem:** teto 2× ainda é um absurdo em MVCC/scan e é frouxo demais nas escritas que já estão na mesma classe de sync.
- **Problem:** “1.1× vs Rocks” sem dizer *qual* Rocks (fdatasync vs `F_FULLFSYNC`) reproduz o alvo impossível do 0031/0032.
- **Problem:** A/B/D/E/C na tabela 4096/2000 estão **velhos** (`07bd443`). C no smoke 1024 já dá 404k qps; B/D são 95% leitura — a remesura pode fechar linhas sem código novo. Sem essa run o gap é hipótese.

## Proposed Solution

1. **Gate oficial:** `compat / rocks_F_FULLFSYNC ≥ 0.91` em **todo** shape da tabela. Sem floor 1.1× de escrita vs fdatasync (G1).
2. **Remesurar primeiro** o par completo no harness atual (`af2c2d5`+). B/D/E/C provavelmente andam só com o que já entrou (janela, get seek, `last_under_prefix`, scan lazy + block cache).
3. **Residual de escrita** (apply / raftlog / o que a remesura ainda mostrar < 0.91): cortar trabalho **à volta** do único `sync_all`, nunca o sync. Conta de syscalls antes de teoria.
4. **Residual de leitura** (C se ainda curto a 4096; E; scan; MVCC): mesmo LSM, menos I/O por seek (L0 sobrepostos, `point_at`+`lookup` duplicado, bloom). Semântica = `lookup` / `range_at`. Sem cap por camada que esconda chave viva.
5. **G1–G8 intactos.** Adversarial sem editar asserção. Sem thread no core. Sem `sync_data`.

## Orçamento (floor = 0.91 × Rocks FF da run)

Remesura P0.2 (`fbe39bf` tree, 4096/2000 zipfian 1 KB, `ROCKS_PARITY_FULL_SYNC=1`, [findings/tikv-ycsb-0034-fullsync](../findings/tikv-ycsb-0034-fullsync/)):

`slower` = Rocks_FF / Pedra. Alvo 1.1× = slower ≤ 1.1 (ratio ≥ 0.91). **0 / 11 passam.**

| shape | Pedra qps | p50 | Rocks FF qps | p50 | ratio | **mais lento** | 1.1× |
|---|---:|---:|---:|---:|---:|---:|---|
| ycsb_a | 401 | 3.58 ms | 449 | 3.60 ms | 0.893 | **1.12×** | não |
| ycsb_b | 3 385 | 6.5 µs | 4 548 | 1.5 µs | 0.744 | **1.34×** | não |
| ycsb_c | 152 728 | 5.7 µs | 1 224 864 | 0.7 µs | 0.125 | **8.0×** | não |
| ycsb_d | 3 789 | 12 µs | 4 770 | 1.9 µs | 0.794 | **1.26×** | não |
| ycsb_e | 2 280 | 0.19 ms | 4 942 | 9 µs | 0.461 | **2.17×** | não |
| ycsb_f | 391 | 3.76 ms | 464 | 3.85 ms | 0.843 | **1.19×** | não |
| deps_apply_batch | 66 | 9.09 ms | 83 | 8.78 ms | 0.798 | **1.25×** | não |
| deps_mvcc_latest | 2 465 | 0.37 ms | 170 195 | 4.2 µs | 0.014 | **69×** | não |
| deps_scan | 5 532 | 0.18 ms | 192 201 | 3.5 µs | 0.029 | **35×** | não |
| deps_raftlog | 105 | 4.92 ms | 189 | 4.16 ms | 0.556 | **1.80×** | não |
| deps_cache_overwrite | 124 | 4.54 ms | 218 | 4.08 ms | 0.567 | **1.76×** | não |

Nada está “dentro”. A/F/overwrite da tabela velha (`07bd443`) **não** se repetiram neste peer (Rocks FF saiu mais rápido). Gate 0.91 em subconjunto verde = conjunto vazio — não ligar um gate mentiroso.

## Garantias invariáveis

Herdadas de RFC-0031 G1–G8. Em especial:

- **G1** — WAL `sync_all` antes do Ok. Este RFC **não** troca por `sync_data` / `fdatasync` para caçar o peer fdatasync.
- **G2** — visibilidade de get/scan/latest = `lookup` / `range_at` (tombstone, seq). Sem cap por camada que esconda chave viva.
- **G4** — `cargo test -p rocksdb-compat` adversarial **sem** editar asserção.
- **G6** — sem thread no core; compact continua no write/flush.
- **G8** — par real, `ROCKS_PARITY_FULL_SYNC=1` rotulado. Floor = 0.91 × **peer da run**.

Editar asserção existente para ficar verde é relaxação — volta para o desenho.

## Delivery slices (mandatory)

### P0 — must ship first (útil sozinho)

- [x] **P0.1** RFC + Status vivo: teto 1.1× em todos os shapes vs FF; fdatasync-write fora do gate — status: `done`
- [x] **P0.2** Remesura completa `tikv_ycsb_parity_v0.sh` + `ROCKS_PARITY_FULL_SYNC=1` (4096/2000 zipfian 1 KB); substituir as linhas velhas de B/C/D/E na tabela — status: `done`
- [ ] **P0.3** Gate `ROCKS_PARITY_RATIO_FLOOR=0.91` nos shapes que a remesura mostrar ≥ 0.91 — status: `todo` (parked: quiet remesure; remesura: **0/11**; não ligar gate vazio)

### P1 — escritas / mix que a remesura ainda deixar < 0.91

- [ ] **P1.1** Conta medida do extra além de um `sync_all` em `deps_apply_batch` (e o pior mix que falhar P0.2) — status: `todo` (parked: quiet remesure)
- [ ] **P1.2** `deps_apply_batch` e `deps_raftlog` ≥ 0.91 vs FF da run — status: `todo` (parked: quiet remesure)
- [ ] **P1.3** ycsb_b / ycsb_d ≥ 0.91 se ainda falharem depois de P0.2 (são 95% leitura: fechar C/E pode bastar) — status: `todo` (parked: quiet remesure)

### P2 — leituras até 0.91

- [ ] **P2.1** ycsb_c ≥ 0.91 a **4096/2000** (não só o smoke 1024) — status: `todo` (parked: quiet remesure)
- [ ] **P2.2** ycsb_e ≥ 0.91 — status: `todo` (parked: quiet remesure)
- [ ] **P2.3** `deps_scan` ≥ 0.91 — status: `todo` (parked: quiet remesure)
- [ ] **P2.4** `deps_mvcc_latest` ≥ 0.91 — status: `todo` (parked: quiet remesure)
- [ ] **P2.5** Gate 0.91 em **todos** os 11 shapes; se algum residual: follow-up com número (L0/seek, blocos, syscalls) — não relaxar G1 — status: `todo` (parked: quiet remesure)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + teto 1.1× documentado | done | este doc | 2026-08-15 |
| P0.2 | p0 | remesura completa 4096/2000 FF | done | findings/tikv-ycsb-0034-fullsync; **0/11** ≥ 0.91 | 2026-08-15 |
| P0.3 | p0 | gate 0.91 nos já-verdes | todo | conjunto vazio — não ligar | 2026-08-15 |
| P1.1 | p1 | conta extra além do sync_all | todo | — | 2026-08-15 |
| P1.2 | p1 | apply + raftlog ≥ 0.91 | todo | — | 2026-08-15 |
| P1.3 | p1 | B/D ≥ 0.91 se ainda falharem | todo | — | 2026-08-15 |
| P2.1 | p2 | ycsb_c ≥ 0.91 @4096 | todo | — | 2026-08-15 |
| P2.2 | p2 | ycsb_e ≥ 0.91 | todo | — | 2026-08-15 |
| P2.3 | p2 | deps_scan ≥ 0.91 | todo | — | 2026-08-15 |
| P2.4 | p2 | deps_mvcc_latest ≥ 0.91 | todo | — | 2026-08-15 |
| P2.5 | p2 | gate 0.91 todos os shapes | todo | — | 2026-08-15 |

## Acceptance Criteria

- **Tests:** adversarial `cargo test -p rocksdb-compat` **sem** editar asserção; `last_under_prefix` / `try_scan_at` limit+tombstone continuam verdes; ycsb_a / F não regridem no smoke do par.
- **Telemetry:** `scripts/tikv_ycsb_parity_v0.sh` + `ROCKS_PARITY_FULL_SYNC=1` + `ROCKS_PARITY_RATIO_FLOOR=0.91`; `meets_floor=true` em **todos** os 11 shapes da tabela (P2.5). Até lá P0.3 gateia só o subconjunto já verde.
- **Documentation:** este RFC + linha em `open-items.md` + tabela em `findings/tikv-ycsb-lab-*.md` no commit da remesura (P0.2).
- **Screenshots:** backend-only.

## Out of scope

- 1.1× (ou 2×) de **escrita** vs fdatasync (G1).
- Trocar `File::sync_all` por `sync_data`.
- `engine_pedra` / cluster TiKV / go-ycsb 3-node.
- Thread de compact no core.
- Afrouxar accept-set, fencing, CRC fail-closed, ou visibilidade de snapshot.
