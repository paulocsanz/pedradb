# RFC-0033: Close MVCC latest + deps_scan to the 2× floor

**Status:** draft  
**Updated:** 2026-08-15  
**Parents:** [0032](0032-tikv-mix-2x-budget.md) (teto 2× nesta tabela, G1–G8), [0031](0031-rocks-parity-10x-budget.md)

## Background

- RFC-0032 trava `Pedra / Rocks_F_FULLFSYNC ≥ 0.5` na tabela TiKV-mix (zipfian, 1 KB). Escritas A/B/F/apply e, após P0.6, **point-get C** e **ycsb_e** já passam. Sobram **dois** shapes absurdo:

| shape | original (`07bd443`) | após 0032 P0.1–P0.6 (`5cf09a9`) | Rocks F_FULLFSYNC | floor 0.5 | gap |
|---|---:|---:|---:|---:|---|
| MVCC latest | 3.2 qps / p50 292 ms | **124 qps / p50 1.9 ms** | 74.605 | 37.303 | ~300× |
| deps_scan (≤25) | 13 qps / p50 62 ms | **1.024 qps / p50 0.97 ms** | 12.961 | 6.481 | ~6× |

- O que **já não é**: copiar o CF no iterator (P0.1), SeekForPrev no CF (P0.2), varrer a memtable no range (P0.5) ou no get (P0.6).
- O que **ainda é** (fonte, `scan_at_raw` + `StreamingVisibleIter::new` + `SstTable::entries_in_user_range`):
  1. **Pré-materializa** um `Vec` por camada (mem + **cada** SST que sobrepõe o prefixo) **antes** do merge e **antes** do `limit`.
  2. Todo L0 sobrepõe o keyspace — 4 arquivos L0 (`L0_COMPACTION_TRIGGER`) + L1 ⇒ todo seek de prefixo decodifica blocos em **todos** os L0.
  3. `latest_cf` ainda é “iterator forward no prefixo + last key”: paga um `scan_at` completo (valores 1 KB resolvidos) para 2–3 versões.
  4. `deps_scan` pede 25 keys visíveis mas o scan já carregou todas as versões do intervalo em todos os arquivos.

Get (C) prova o ponto: o mesmo LSM, seek de BTree no mem, 405k qps. MVCC/scan não podem continuar 2 ms/op.

## Problems This Solves

- **Problem:** MVCC latest ~300× abaixo do floor 2× — inútil para um apply/read path estilo TiKV.
- **Problem:** deps_scan ~6× abaixo do floor; o `limit=25` não corta I/O porque o limite é no emit, não na coleta.
- **Problem:** sem um `last_in_range` por arquivo, “latest” é um scan genérico caro.

## Proposed Solution

1. **`last_in_range` / `last_under_prefix` no core** (P0): por camada, só a última user-key no intervalo (mem: `BTree` `next_back` no range; SST: último ponto no(s) bloco(s) que tocam o fim do prefixo). Merge = max das camadas. Sem `StreamingVisibleIter`, sem resolver 1 KB de versões velhas. `latest_cf` passa a chamar isso.
2. **`limit` corta coleta, não só emit** (P0): `try_scan_at` / `entries_in_user_range` paramam de decodificar blocos quando o merge já emitiu `limit` user-keys visíveis (deps_scan = 25).
3. **Menos L0 sobrepostos no seek** (P1): compact L0→L1 já existe (`L0_COMPACTION_TRIGGER=4`); o residual é decodificar 4 L0. Bloom/bounds por prefixo no SST + não construir `Vec` do arquivo inteiro se o bloco seguinte está fora. Sem baixar `sync_all`.
4. **G1–G8 intactos.** Adversarial do compat sem editar asserção. Semântica latest = maior user-key com o prefixo (igual hoje).

## Orçamento

Mesmos knobs da tabela 0032 (`tikv_ycsb_parity_v0.sh`, `ROCKS_PARITY_FULL_SYNC=1`):

| shape | floor (qps) | critério |
|---|---:|---|
| `deps_mvcc_latest` | 37.303 | ≥ 0.5 × Rocks FF da mesma run (re-medida no PR) |
| `deps_scan` | 6.481 | idem |
| ycsb_e | já ≥ | não regredir abaixo do floor 0032 |

Re-medir no mesmo harness; se o peer FF desta máquina mudar, o floor é **0.5 × peer da run**, não o número 37k cravado para sempre.

## Garantias invariáveis

Herdadas de RFC-0031/0032. Em especial:

- **G1** — WAL `sync_all` antes do Ok. Este RFC é só read-path.
- **G2** — latest/scan no snapshot: mesma visibilidade que `range_at` (tombstone, seq).
- **G4** — `cargo test -p rocksdb-compat` adversarial sem mudar asserção.
- **G6** — sem thread; compact continua no write/flush (já existe).
- **G8** — números com o par real, FULL_SYNC rotulado.

## Delivery slices (mandatory)

### P0 — must ship first (useful alone)

- [ ] **P0.1** `Db::last_under_prefix(seq, prefix)` / `last_in_range` — mem BTree + por-SST last point no prefixo; teste: K versões × N users, devolve a maior do user, sem ver o vizinho — status: `todo`
- [ ] **P0.2** `latest_cf` (compat bench + engines) usa `last_under_prefix`; re-medida `deps_mvcc_latest` — status: `todo`
- [ ] **P0.3** `try_scan_at` honra `limit` na coleta (para de puxar blocos/streams quando `emitted == limit`); `deps_scan` re-medido — status: `todo`
- [x] **P0.4** RFC + Status vivo (este doc) — status: `done`

### P1 — next wave

- [ ] **P1.1** SST: `entries_in_user_range` / last-in-prefix não decodifica bloco cujo `first_user_key` já passou do end — status: `todo`
- [ ] **P1.2** Gate `ROCKS_PARITY_RATIO_FLOOR=0.5` + `ROCKS_PARITY_GATE_SHAPES=deps_mvcc_latest,deps_scan` contra FULL_SYNC no `tikv_ycsb_parity_v0.sh` — status: `todo`

### P2 — later / polish

- [ ] **P2.1** Atualizar a tabela em `findings/tikv-ycsb-lab-*.md` com o commit da re-medida — status: `todo`
- [ ] **P2.2** Se ainda < 0.5: follow-up com número (L0 count, blocos decodificados/seek) — não relaxar G1 — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | last_under_prefix no core | todo | — | 2026-08-15 |
| P0.2 | p0 | latest_cf usa last_under_prefix | todo | — | 2026-08-15 |
| P0.3 | p0 | scan limit corta coleta | todo | — | 2026-08-15 |
| P0.4 | p0 | RFC + status vivo | done | este doc | 2026-08-15 |
| P1.1 | p1 | SST skip blocos past end | todo | — | 2026-08-15 |
| P1.2 | p1 | gate 0.5 mvcc+scan | todo | — | 2026-08-15 |
| P2.1 | p2 | tabela lab | todo | — | 2026-08-15 |
| P2.2 | p2 | follow-up se residual | todo | — | 2026-08-15 |

## Acceptance Criteria

- **Tests:** unit `last_under_prefix` (vários users × versões, snapshot mid, tombstone); `try_scan_at(..., Some(25))` não observa mais de 25 user-keys visíveis e um teste de instrumentação (contador de blocos decodificados, ou assert de que um SST fora do range não é aberto) prova que o limit não é só no emit; `cargo test -p rocksdb-compat` adversarial **sem** editar asserção; ycsb_c / A / F não regridem no smoke do par.
- **Telemetry:** `scripts/tikv_ycsb_parity_v0.sh` + FULL_SYNC=1; report `deps_mvcc_latest` e `deps_scan` `meets_floor=true` com floor 0.5.
- **Documentation:** este RFC + linha em `open-items.md` + findings no commit do P0.2/P0.3.
- **Screenshots:** backend-only.

## Out of scope

- 2× de escrita vs fdatasync (RFC-0032).
- `engine_pedra` / cluster TiKV.
- Trocar `File::sync_all` por `sync_data`.
- Thread de compact no core (L0 compact no flush já existe).
- Refazer o iterator do compat (já tem janela); este RFC é o **read path do core** por baixo.
