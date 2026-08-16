# RFC-0036: ≤2× vs TiKV-class Rocks (`fdatasync`) — WAL still synced before Ok

**Status:** in-progress  
**Updated:** 2026-08-16  
**Parents:** [0001](0001-pedradb-high-level-spec.md) (O1: WAL `fdatasync` before Ok), [0031](0031-rocks-parity-10x-budget.md)–[0035](0035-mvcc-scan-2x-measure-first.md) (same-class FF table)

## Background

- Pedra no Ok faz `File::sync_all`. No Apple, Rust std mapeia **tanto** `sync_all` **quanto** `sync_data` para `fcntl(F_FULLFSYNC)` (~4–5 ms neste Mac).
- O Rocks que o TiKV usa (`sync-log=true` → `WriteOptions.sync=true`) chama `libc::fdatasync` (~30–50 µs). `librocksdb-sys` desta build **não** define `HAVE_FULLFSYNC`.
- Remesura `38cef53`, 4096/2000 zipfian 1 KB, ycsb+deps: leituras já ≤2× (C 1.39×; MVCC/scan **0.77×**). Escritas 27–191× — o piso é o syscall, não o LSM.
- RFC-0001 O1 já é **`fdatasync` antes do Ok**. A linha posterior “não trocar `sync_all` por `sync_data`” assumia que `File::sync_data` era `fdatasync`. Neste Mac **não é**.

## Problems This Solves

- **Problem:** 2× vs o Rocks padrão do TiKV é o alvo; com `F_FULLFSYNC` vs `fdatasync` as escritas são fisicamente ~100×.
- **Problem:** descer o WAL para “não sincar” quebraria G1. Precisamos do **mesmo** barrier class do peer, não da ausência de barrier.

## Proposed Solution

1. `EnvFile::sync_data` no Unix chama `fdatasync(2)` de verdade (`rustix`, sem `unsafe` no core). Apple `F_FULLFSYNC` fica só em `sync_all` (SST / MANIFEST / CHANGELOG / close).
2. `commit_ops_with` / `wal_sync_group` usam `wal.sync_data()`. Append → sync → Ok. Falha de sync ainda **cerca** o `Db` (RFC-0015 H1). CRC, atomicidade, fail-closed intactos.
3. Gate 2× = `compat / rocks_fdatasync ≥ 0.5` na tabela de 11 shapes. Sem floor de escrita vs um peer que não sinca.

## Garantias invariáveis

| # | Garantia | Este RFC |
|---|---|---|
| G1 | WAL sincado antes do Ok | **mantida** — `fdatasync` (RFC-0001), não skip |
| G2 | visibilidade = lookup / range_at | intocada |
| G4 | adversarial compat | re-verde **sem** editar asserção |
| G5 | fence se sync falha depois do append | `sync_data` Err → `DurabilityFenced` |
| G6 | sem thread no core | intocada |
| G8 | números honestos | peer = Rocks `sync=true` / `fdatasync`; label no JSON |

Não é “menos durável que o TiKV”. É a **mesma** classe. `F_FULLFSYNC` continua disponível em `sync_all` para ficheiros publicados.

## Delivery slices (mandatory)

### P0 — must ship first

- [x] **P0.1** RFC + Status vivo (este doc) — status: `done`
- [x] **P0.2** `EnvFile::sync_data` = `fdatasync(2)` no Unix; WAL commit/group usa `sync_data`; fence inalterado — status: `done`
- [ ] **P0.3** Remesura 11/11 ≥ 0.5 vs Rocks fd — status: `doing` (8/11; faltam apply / raftlog / overwrite-variância — compact inline)

### P1 — next wave

- [ ] **P1.1** Gate `ROCKS_PARITY_RATIO_FLOOR=0.5` no script **sem** FULL_SYNC (todas as 11) — status: `todo`
- [x] **P1.2** CHANGELOG fora do caminho do commit (interval 0; flush/close ainda persistem) — status: `done`

### P2 — later

- [ ] **P2.1** Coluna FF continua report-only (opt-in `sync_all` no WAL) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este doc | 2026-08-16 |
| P0.2 | p0 | WAL `fdatasync` + fence | done | `pedradb-posix` + commit `sync_data` | 2026-08-16 |
| P0.3 | p0 | remesura 11/11 ≥ 0.5 vs fd | doing | 8/11; apply/raftlog compact inline | 2026-08-16 |
| P1.1 | p1 | gate 0.5 default Rocks | todo | — | 2026-08-16 |
| P1.2 | p1 | CHANGELOG fora do commit | done | interval default 0 | 2026-08-16 |
| P2.1 | p2 | FF report-only | todo | — | 2026-08-16 |

## Acceptance Criteria

- **Tests:** `cargo test -p rocksdb-compat` adversarial **sem** editar asserção; `sync_fail_after_append_fences_until_reopen` continua verde (FailingEnv já intercepta `sync_data`).
- **Telemetry:** `scripts/tikv_ycsb_parity_v0.sh` (default FULL_SYNC=0); finding com p50/p95/p99; `meets_floor` nas 11 após P0.3.
- **Documentation:** este RFC + tabela em `findings/tikv-ycsb-lab-20260815.md`. backend-only.
- **Screenshots:** none — backend-only.

## Out of scope

- Skip de sync no Ok.
- 2× vs Rocks `sync=false` (async WAL).
- Trocar `F_FULLFSYNC` em SST/MANIFEST (não é o caminho do put).
