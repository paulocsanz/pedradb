# RFC: 0065 — Column families físicas: N LSM, 1 WAL

**Status:** in-progress
**Updated:** 2026-08-27
**Parents:** [0062](0062-launch-readiness-remaining-gaps.md) P2.3,
[0009](0009-rocksdb-class-engine.md),
[0047](0047-compat-dropin-failure-profile.md)
**Evidence:** [`findings/2026-08-27-tikv-physical-cf`](../../findings/2026-08-27-tikv-physical-cf/README.md)

**Host:** TiKV. Surreal 1.5 não precisa. Não é `patch` no crates.io `rocksdb` 0.22 — TiKV liga `tikv/rust-rocksdb`.

## Background

- Compat hoje: `cf\0key` **num** LSM. `get_cf("lock")` não vê `write` (S2). Flush, compact, L0, stall, cache são **globais**.
- TiKV kvdb: CFs Rocks **físicos** `default` / `write` / `lock`. Cada um tem memtable, SST, compact, L0. **Um** WAL. **Um** `WriteBatch` (prewrite/commit Percolator = lock+default+write no mesmo fsync). Raftdb é **outro** `DB` (log Raft) — isso já é 2 instâncias, não 3 CFs.
- TiKV **não** reimplementa CF em cima de um Rocks só. Pede ao C++.
- 3 `ConcurrentDb` (3 pastas, 3 WAL) **não** é o modelo: apply deixa de ser atómico; crash no meio = lock committed, default não = silent-wrong no 2PC.
- UDT: TiKV **não** usa (ts na *user key*). Titan: plugin do fork PingCAP; vlog já é a ideia; API Titan fora deste RFC.
- `write()` do compat já é um `apply_batch_vec` = um grupo WAL. Isso **fica**. O que muda é para onde o flush/compact mandam os bytes.

## Problems This Solves

- **Problem:** compact/flush do `default` (gordo) reescreve o `lock` (magro). TiKV dimensiona e compacta por CF; o prefixo torna isso um no-op.
- **Problem:** stall/L0 globais — write no default para o lock (e o contrário).
- **Problem:** “abrir 3 DBs” parece barato e quebra a atomicidade que o apply do TiKV assume.

## Proposed Solution

Mesmo contrato do Rocks CF:

1. **1 WAL, 1 MANIFEST, 1 `WriteBatch` atómico** (já é o `write()` de hoje).
2. **N florestas de SST** (P0): flush emite um SST por CF que teve keys; compact só lê a família.
3. **N memtables + write_buffer / L0 por CF** (P1): flush do lock não despeja o default.
4. Raftdb = segundo `DB` num path (P2), como o TiKV já faz — **não** CF do kvdb.

On-disk antigo (SST prefix-misturado) continua a abrir; o próximo flush/compact parte. Sem job de migração.

## Delivery slices (mandatory)

### P0 — must ship first (SST por CF; WAL e batch iguais)

- [x] **P0.1** SST + MANIFEST conhecem o CF; flush de uma memtable mista emite **um ficheiro por CF com keys**; SST prefix-era abre como família `default`/misturada — status: `done`
- [x] **P0.2** `compact` / `compact_range_cf` só reescreve SST daquela família; teste: compact `lock` não muda bytes/contagem de SST `default` — status: `done`
- [x] **P0.3** Dente de atomicidade: `WriteBatch` lock+default+write, kill no meio do commit, recover all-or-nothing (já WAL único; o teste **recusa** o desenho 3-DB) — status: `done`

P0 sozinho já é útil: compact do lock deixa de varrer o default. Flush ainda é da memtable partilhada.

### P1 — next wave (memtable / stall por CF)

- [x] **P1.1** Memtable (e `write_buffer_size` de `ColumnFamilyDescriptor`) **por CF**; auto-flush só da família que passou o limite — status: `done`
- [x] **P1.2** L0 / stop-writes **por CF** (default gordo não stall o lock) — status: `done`
- [x] **P1.3** Prova: `deps_lock_prewrite` (ou unit) — flush do lock **não** cria SST `default`; recover do mesmo WAL povoa as N memtables — status: `done`

### P2 — later

- [ ] **P2.1** Segundo `DB` no path do raftdb (log Raft); kvdb não mistura `raft` como CF prefixo — status: `todo`
- [ ] **P2.2** Cache/block por CF (opcional; default partilhado continua correcto) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | SST tagged by CF; split flush | done | MANIFEST v5 + `write_imm_l0_files` | 2026-08-27 |
| P0.2 | p0 | compact_range_cf only that family | done | `compact_ssts_only_cf` / `live_files` | 2026-08-27 |
| P0.3 | p0 | multi-CF batch crash all-or-nothing | done | `multi_cf_batch_crash_recovers_all_or_nothing` | 2026-08-27 |
| P1.1 | p1 | per-CF memtable + write_buffer | done | `take_family` / `flush_cf` / `set_cf_write_buffer` | 2026-08-27 |
| P1.2 | p1 | per-CF L0 stall | done | `ensure_write_admitted_for` | 2026-08-27 |
| P1.3 | p1 | lock flush ≠ default SST | done | `flush_cf_lock_does_not_create_default_sst` | 2026-08-27 |
| P2.1 | p2 | raftdb second DB path | todo | — | 2026-08-27 |
| P2.2 | p2 | per-CF cache | todo | — | 2026-08-27 |

## Acceptance Criteria

- **Tests**
  - P0.1: flush com keys em `lock` e `default` → ≥1 SST `lock` e ≥1 SST `default`; reopen lê os dois `get_cf`.
  - P0.1: DB só-prefixo (pré-0065) abre e `get_cf` continua certo.
  - P0.2: `compact_range_cf("lock")` não altera `LiveFile` / bytes do `default`.
  - P0.3: batch 3 CFs + `FailingEnv` no sync → recover vê as 3 keys ou nenhuma; fixture “3 DBs / 3 WAL” **não** é o desenho (doc + teste de contrato: um `apply_batch`).
  - P1.1: `write_buffer` do lock pequeno; puts só no default **não** flusham lock.
  - P1.2: L0 do default no stop-trigger; put no lock ainda `Ok` (até o lock bater o próprio trigger).
- **Telemetry / Analytics:** none no P0 — invariante de layout. P1 pode expor `sst_count` por CF (já há `column_family_metadata` no compat).
- **Documentation:** este RFC; 0062 P2.3 aponta para cá; finding physical-cf actualizado quando P0 fechar.
- **Screenshots:** backend-only.

## Out of scope

- 3 `ConcurrentDb` / 3 WAL para `default`/`write`/`lock`.
- Titan knobs / `DBTitanDBBlobRunMode`.
- Rocks UDT / comparator de timestamp (TiKV ts vai na user key).
- `engine_traits` do TiKV (integração do crate no binário TiKV). Isto é o **engine**.
- Relitigar coluna A (`sync=false`) ou Darwin B.
- Extrair `db.rs`.
