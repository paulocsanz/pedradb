# RFC-0032: 2× budget on the tabulated TiKV-mix workloads

**Status:** draft  
**Updated:** 2026-08-24
**Parked (quiet remesure):** remaining P-slices need a quiet 3× host; dirty sandbox numbers are not the official floor (AGENTS.md peer `sync=false`).  
**Parents:** [0031](0031-rocks-parity-10x-budget.md) (classe de sync + G1–G8), [rocksdb-compat](../rocksdb-compat.md), [lab table](../findings/tikv-ycsb-lab-20260815.md)

## Background

- Baseline pinned (lab 2026-08-15, `07bd443`, zipfian, 1 KB, 4096 records, 2000 ops, single-thread, **not** a 3-node TiKV cluster):

| shape | Pedra | Rocks fdatasync | ratio | Rocks F_FULLFSYNC | ratio |
|---|---:|---:|---:|---:|---:|
| A 50/50 | 329 | 18.437 | 0.018 | 339 | **0.97** |
| B 95/5 | 2.096 | 113.784 | 0.018 | 3.149 | **0.67** |
| C 100% get | 14.909 | 460.454 | 0.032 | 373.178 | 0.040 |
| F RMW | 345 | 8.735 | 0.040 | 332 | **1.04** |
| apply batch | 43 | 1.194 | 0.036 | 61 | **0.70** |
| MVCC latest | 3.2 | 107.259 | ~0 | 74.605 | ~0 |
| short scan (E / coproc) | 28 / 13 | 36k / 7k | ~0 | 3k / 13k | ~0 |

- Duas colunas Rocks **não são o mesmo contrato**. Neste Mac: `fdatasync` p50 ~50 µs; `F_FULLFSYNC` (`File::sync_all`) p50 ~4.8 ms (~100×). O rust-rocksdb 0.22 / `librocksdb-sys` desta build **não** define `HAVE_FULLFSYNC` — `WriteOptions.sync=true` é fdatasync. Pedra no Ok faz `sync_all` = `F_FULLFSYNC`.
- Escritas vs F_FULLFSYNC **já estão dentro de 2×** (várias ≥1×). O buraco desta tabela é **leitura**: point-get ~25×, MVCC/scan ~0 (iterador eager, p50 latest 292 ms).
- RFC-0031 cobriu debounce do CHANGELOG e o peer `ROCKS_PARITY_FULL_SYNC`. Este RFC trava o orçamento **2× nesta tabela**, sem relaxar G1.

## Problems This Solves

- **Problem:** “2× vs Rocks” sem dizer *qual* Rocks — fdatasync vs F_FULLFSYNC — produz um alvo impossível nas escritas ou um alvo já cumprido, os dois com a mesma frase.
- **Problem:** MVCC latest e short scan desta tabela são inutilizáveis (~3 qps / ~13–28 qps) por materializar o CF inteiro; isso é o que um workload estilo TiKV sente.
- **Problem:** point-get (C) está ~25× atrás nos **dois** peers — não é classe de sync (get não fsynca).

## Proposed Solution

1. **Dois gates, nunca misturados.**
   - **Gate oficial (esta tabela):** `compat / rocks_F_FULLFSYNC ≥ 0.5` em **todo** shape. Escritas A/B/F/apply já passam; C + MVCC + scan são o trabalho.
   - **Coluna fdatasync:** report-only nas **escritas** (física: 1× `F_FULLFSYNC`/put vs ~50 µs). Nas **leituras** (C, MVCC, scan) o 2× vs fdatasync **é o mesmo trabalho** que vs F_FULLFSYNC — get/iter não pagam sync.
2. **P0 fecha MVCC + scan** com iterador de janela + latest por prefixo (não SeekForPrev no CF inteiro). Semântica do iterator testada hoje permanece; adversarial **sem editar asserção**.
3. **P1 fecha point-get (C)** até ≥ 0.5 vs F_FULLFSYNC (~186k qps de floor nesta run — ou re-medida no mesmo harness). Sem baixar `sync_all`.
4. **G1–G8 do RFC-0031 aplicam-se intactos.** Editar asserção existente para ficar verde é relaxação.

## Orçamento (floor = peer / 2)

| shape | floor vs F_FULLFSYNC | Pedra hoje | vs floor | o que falta |
|---|---:|---:|---|---|
| A 50/50 | 170 | 329 | **já ≥** | — |
| B 95/5 | 1.575 | 2.096 | **já ≥** | — |
| F RMW | 166 | 345 | **já ≥** | — |
| apply batch | 31 | 43 | **já ≥** | — |
| C 100% get | 186.589 | 14.909 | 12.5× curto | caminho de get (mutex? cópia? bloom?) |
| MVCC latest | 37.303 | 3.2 | ~10⁴× curto | iterador eager / SeekForPrev no CF |
| short scan E | ~1.5k / ~6.5k | 28 / 13 | ~10²–10³× curto | mesmo iterador, janela 25 |

Não existe linha de floor 2× **de escrita** vs fdatasync neste RFC. Afirmar isso exigiria `sync_data` no Ok (quebra G1 neste Mac) ou group-commit multi-thread (fora do harness single-client desta tabela).

## Garantias invariáveis

Herdadas de [RFC-0031](0031-rocks-parity-10x-budget.md) G1–G8. Em particular:

- **G1** — `wal.sync_all()` antes do Ok. Este RFC **não** troca por `sync_data` para caçar o peer fdatasync.
- **G4** — suite adversarial do compat re-verde sem editar asserção (iterator positioning ×8 incluso).
- **G6** — sem thread no core; janela do iterador é pull-driven.
- **G8** — re-medir com `scripts/tikv_ycsb_parity_v0.sh` (mesmos knobs: zipfian, 1 KB, 4096/2000) + `ROCKS_PARITY_FULL_SYNC=1`.

## Delivery slices (mandatory)

### P0 — must ship first (useful alone)

- [x] **P0.1** Iterador do compat com janela bornada no *forward* (`range_at_limited` / refill; `collect_rest` recarrega até o bound do CF) — status: `done`
- [x] **P0.2** `latest_cf` / SeekForPrev-shaped: last key em `[prefix, prefix_succ)` — **não** reverse-scan do CF inteiro — status: `done`
- [ ] **P0.3** Re-medir esta tabela (`tikv_ycsb_parity_v0.sh` + FULL_SYNC=1); MVCC latest e short scan ≥ floor 0.5 vs F_FULLFSYNC; adversarial iterator + `cargo test -p rocksdb-compat` sem editar asserção — status: `todo` (parked: quiet remesure; P0.1/P0.2: mvcc 3.2→81 qps / p50 292→2.3 ms @1024/1KB)
- [x] **P0.4** RFC + Status vivo (este doc) — status: `done`
- [x] **P0.5** MemTable `iter_internal_range` (BTree `range`, sem varrer o mapa) no `memtable_stream` quando não há range-tombstone — status: `done`
- [x] **P0.6** `Db::lookup` point-get via `MemTable::get_entry` (seek) em vez de `iter_internal` linear — status: `done`

### P1 — next wave

- [x] **P1.1** Point-get (shape C) ≥ 0.5 vs F_FULLFSYNC nesta tabela — status: `done` (`lookup` usava scan linear da memtable; agora `get_entry` BTree. Lab 1024/1KB zipfian: ycsb_c **404k qps** / p50 2 µs ≥ floor 186k)
- [ ] **P1.2** Gate `ROCKS_PARITY_RATIO_FLOOR=0.5` em **todos** os shapes desta tabela contra o peer FULL_SYNC; `tikv_ycsb_parity_v0.sh` documenta o comando — status: `todo` (parked: quiet remesure)

### P2 — later / polish

- [ ] **P2.1** Atualizar a tabela em `findings/tikv-ycsb-lab-*.md` + `rocksdb-compat.md` com o commit da re-medida — status: `todo` (parked: quiet remesure)
- [ ] **P2.2** Se C ou scan ainda < 0.5: follow-up com mecanismo novo e número (não engessar; não relaxar G1) — status: `todo` (parked: quiet remesure)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | iterator janela forward | done | este commit | 2026-08-15 |
| P0.2 | p0 | latest_cf prefix-bounded | done | este commit | 2026-08-15 |
| P0.3 | p0 | re-medida MVCC/scan ≥ 0.5 | todo | — | 2026-08-15 |
| P0.4 | p0 | RFC + status vivo | done | este doc | 2026-08-15 |
| P0.5 | p0 | memtable range prune | done | fd2fb96 | 2026-08-15 |
| P0.6 | p0 | lookup get_entry seek | done | este commit | 2026-08-15 |
| P1.1 | p1 | point-get C ≥ 0.5 | done | este commit | 2026-08-15 |
| P1.2 | p1 | gate 0.5 all-shapes FULL_SYNC | todo | — | 2026-08-15 |
| P2.1 | p2 | tabela lab atualizada | todo | — | 2026-08-15 |
| P2.2 | p2 | follow-up se residual | todo | — | 2026-08-15 |

## Acceptance Criteria

- **Tests:** `cargo test -p rocksdb-compat` (API + adversarial, **zero asserções editadas**); teste novo: N≫WINDOW keys, `From(mid, Forward).collect_rest()` igual ao modelo; `latest` de um prefixo com K versões devolve a de maior sufixo sem varrer outro prefixo.
- **Telemetry:** `scripts/tikv_ycsb_parity_v0.sh` + compare com `ROCKS_PARITY_FULL_SYNC=1` e `ROCKS_PARITY_RATIO_FLOOR=0.5`; após P0.3 o gate **write+MVCC+scan** passa (C pode esperar P1.1).
- **Documentation:** este RFC + linha em `open-items.md` + findings da re-medida no mesmo commit do P0.3.
- **Screenshots:** backend-only.

## Out of scope

- 2× de **escrita** vs a coluna fdatasync (impossível neste Mac com G1 + single-thread).
- Cluster TiKV / go-ycsb / `engine_rocks` (`engine_pedra` é outro RFC).
- Trocar `File::sync_all` por `sync_data`.
- Thread de background, ingest, compaction filters, `delete_files_in_range`.
