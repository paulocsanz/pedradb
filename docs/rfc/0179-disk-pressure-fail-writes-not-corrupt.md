# RFC-0179 — Disk pressure: recusar writes, não corromper

**Status:** in-progress (P0+P1.1+P1.2+P1.4+P1.5 done; P1.3 / P2 open)
**Updated:** 2026-09-09
**ID:** 0179
**Parents:** [0050](0050-nine-axis-robustness.md) (ENOSPC mid-flush já cerca),
[0173](0173-bounded-cache-ram-degrade.md) (`DONTNEED` liberta RAM, não disco)
**Peer:** não é claim de paridade Rocks. G1 inalterado.

Este RFC é autónomo. Não pisa o [0176](0176-modelo-matematico-de-escala.md)
(modelo de um processo) nem o [0175](0175-sql-cluster-multiwrite-range-raft.md)
(SQL cluster, draft).

## Background

- ENOSPC **a meio** de um flush já cerca o handle (RFC-0050 P0.3). O furo
  é chegar a **0 bytes livres no append do WAL**: o write falha no meio do
  record, recovery vê cauda rasgada, e compact/MANIFEST não têm sítio para
  um SST novo.
- Compact precisa de *headroom*. Recusar **antes** de 0 é o que deixa o
  motor compactar e servir reads.
- `DONTNEED` (0173) larga page cache / RAM. Não aumenta `f_bavail`.
  Reclaim que liberta **disco** é compact / recycle de WAL / GC de vlog /
  delete de SST obsoleto.
- Probe falhou (`statvfs` Err, layout errado, Env de teste sem disco) **não
  é disco cheio**. Unknown → não recusar. ENOSPC a meio do write continua
  a cercar (0050).

## Problems This Solves

- **Problem:** falta de disco no WAL append corrompe ou cerca o universo
  inteiro, em vez de recusar o request que ia escrever.
- **Problem:** compact precisa de espaço; esperar por 0 bytes é tarde.
- **Problem:** o operador não vê o evento (print ungated é RFC-0169; tem
  de ser `tracing`).

## Proposed Solution

Watermarks proactivos no `Env`:

| piso | default | acção |
|------|---------|--------|
| soft | 256 MiB (`DISK_SOFT_FREE_BYTES`) | compact (só se ainda ≥ hard) + `DONTNEED` em `.sst`/`.log`/`.vlog`; re-probe; write segue se ainda ≥ hard |
| hard | 64 MiB (`DISK_HARD_FREE_BYTES`) | `CoreError::DiskPressure { available, need }` — **sem** WAL append, **sem** fence de durabilidade; **reads ficam de pé** |
| unknown (`None`) | probe Err ou Env sem disco | não recusar; 0050 cobre ENOSPC real |

`Env::available_bytes` default `Ok(None)`. POSIX: `statvfs` `f_bavail *
f_frsize` em `pedradb-posix`. Probe `Err` → `None` (nunca “full”).

Admit **incondicional** no write path (`write_admission_idle()` só salta
mem/L0). `DiskPressure` **não** entra no retry de stall do
`ConcurrentDb`. `group_admit` propaga o tipo, não o mapeia a `Internal`.

Log: `tracing::warn!` na **transição** Ok→Reclaim e Ok/Reclaim→Refuse
(não em cada put). RFC-0169: sem `println` ungated.

PITR e replica HA **não** passam pelo `put`. Copiam dest / appendam WAL
cru. O mesmo piso: `admit_disk_write` **antes** de `copy_db_directory`,
`create_checkpoint`, `ship_wal`, replay WAL do restore, e
`append_wal_bytes`. `catch_up` admite **antes** do `pull` para o cursor
não saltar bytes que o replica nunca recebeu. Reclaim no dest vazio é
admitido (não há compact); só o hard recusa.

## Delivery slices (mandatory)

### P0 — recusar write abaixo do hard; get continua; log da transição

- [x] **P0.1** Este RFC + kernel `disk_pressure_admit` (unknown=Ok,
      soft=Reclaim, hard=Refuse) + dentes — status: `done`
- [x] **P0.2** `Env::available_bytes` + POSIX `statvfs` + StdEnv /
      IoUringEnv (Err→None) — status: `done`
- [x] **P0.3** Admit incondicional nos writes; `put` abaixo do hard é
      `DiskPressure`; `get` Ok; `!is_durability_fenced()`; compact no
      reclaim só ≥ hard — status: `done`
- [x] **P0.4** `tracing::warn!` na transição reclaim/refuse (rate-limit
      por estado, não por put) — status: `done`

### P1 — inject + mais reclaim + PITR/HA

- [x] **P1.1** `FailingEnv` inject de `available_bytes` — status: `done`
- [x] **P1.2** WAL recycle / vlog GC no reclaim (além de compact SST) —
      status: `done`
- [ ] **P1.3** sonda de telemetria (RFC-0169, default off) — status: `todo`
- [x] **P1.4** PITR: `restore_pitr` / `ship_wal` / `create_checkpoint` /
      `copy_db_directory` / history restore recusam abaixo do hard;
      dest não fica a meio — status: `done`
- [x] **P1.5** Replica HA: `append_wal_bytes` + `catch_up` recusam abaixo
      do hard; cursor não avança se o append não correu — status: `done`

### P2 — later

- [ ] **P2.1** blast radius do fence 0050 (cópia vs handle) — **não**
      este slice; open-items §2.6 — status: `todo`
- [ ] **P2.2** twin Verus do kernel — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + kernel + dentes | done | disk_pressure_kernel.rs | 2026-09-07 |
| P0.2 | p0 | Env + posix statvfs | done | pedradb-posix / StdEnv | 2026-09-07 |
| P0.3 | p0 | admit + put hard-floor | done | db.rs SpaceEnv test | 2026-09-07 |
| P0.4 | p0 | tracing warn na transição | done | disk_pressure_log AtomicU8 | 2026-09-07 |
| P1.1 | p1 | FailingEnv inject | done | failing.rs set_available_bytes | 2026-09-09 |
| P1.2 | p1 | WAL/vlog reclaim | done | disk_pressure_reclaim_plan | 2026-09-09 |
| P1.3 | p1 | telemetry probe | todo | — | 2026-09-07 |
| P1.4 | p1 | PITR dest/ship recusa hard | done | ops restore_pitr / ship_wal | 2026-09-07 |
| P1.5 | p1 | HA replica append recusa hard | done | replicate append_wal_bytes | 2026-09-07 |
| P2.1 | p2 | fence blast (cópia) | todo | §2.6 | 2026-09-07 |
| P2.2 | p2 | Verus twin | todo | — | 2026-09-07 |

## Acceptance Criteria

- **Tests**
  - Kernel: unknown → Ok; ≥soft → Ok; hard..soft → Reclaim; <hard →
    Refuse; AS-IS sempre Ok; compact forbidden abaixo do hard.
  - `put_under_hard_floor_is_disk_pressure_get_ok`: put Ok com espaço;
    inject abaixo do hard; put seguinte `DiskPressure`; get da primeira
    chave Ok; `!is_durability_fenced()`.
  - `filesystem_available_bytes_temp_dir_nonzero` (posix).
  - `posix_unsafe_rc_sites_all_gated` inclui `statvfs(` e gate `rc != 0`.
  - `pitr_restore_under_hard_floor_is_disk_pressure`: dest vazio / ausente.
  - `ship_wal_under_hard_floor_is_disk_pressure`: watermark inalterado, 0 warch.
  - `replica_append_under_hard_floor_is_disk_pressure`: WAL não cresce.
  - `catch_up_under_hard_floor_does_not_skip_cursor`: offset igual.
  - `failing_env_probe_err_does_not_refuse_put`: probe Err → unknown → put Ok.
  - `restore_history_under_hard_floor_does_not_create_dest`: dest ausente.
  - `concurrent_db_apply_batch_under_hard_floor_is_disk_pressure`: group_admit propaga o tipo, sem retry de stall.
  - `failing_env_delete_under_hard_floor_is_disk_pressure`: sem tombstone.
  - `concurrent_db_probe_err_does_not_refuse_put`: probe Err no write-group ainda admite.
  - `failing_env_compact_under_hard_floor_is_disk_pressure`: compact recusa; get Ok.
- **Telemetry / Analytics:** `tracing::warn!` na transição (não cada put).
  Sonda 0169 é P1.3.
- **Documentation:** este RFC; linha em `docs/status.md`. Não reescreve
  0176.
- **Screenshots:** backend-only.

## Out of scope

- Substituir range Raft / reabrir L9/L29/L34/L40 / implementar 0175.
- Claim de paridade Rocks. Peer oficial continua `sync=false`.
- Expandir o fence 0050 de handle inteiro para “só a cópia” (P2.1).
- `println` ungated / `zero_damaged_pages` / servir snapshot corrupto.
- Recusar writes quando o probe é `None` (falso-positivo em disco são).
