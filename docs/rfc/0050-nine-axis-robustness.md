# RFC: 0050 — Nove eixos de robustez (P0 operável, sem fingir “completo”)

**Status:** in-progress
**Updated:** 2026-08-24
**Parents:** [0016](0016-pedradb-production-robustness.md), [0020](0020-synthetic-field-maturity.md), [0021-tls](0021-security-tls-baseline.md), [0038](0038-wal-corruption-recovery-open-decision.md), [0045](0045-multi-writer-async-5x.md), [0047](0047-compat-dropin-failure-profile.md)
**Scoreboard:** [`../robustness-nine-axes.md`](../robustness-nine-axes.md)

## Background

- O produto tem lab forte (FailingEnv, DST, Darwin class, io_uring soak, group commit, majority lab) e **zero** anos de campo.
- Nove eixos de “operar a sério” estavam abertos: campo, disco Linux, ENOSPC/compact, teto de escrita, Montanha TLS, ops, WAL mid-corrupt, face Rocks/TiKV, encrypt/TLS.
- “Completo” sem campo / plug-pull / firmware é marketing. Esta onda fecha o que **dá para programar** e deixa as paredes escritas.

## Problems This Solves

- **Problem:** não há um scoreboard honesto por eixo — fácil vender GA / TiKV / field peer.
- **Problem:** io_uring soak não injecta CQE `res<0` nem envolve `FailingEnv`.
- **Problem:** ENOSPC/EIO a meio de flush/compact/MANIFEST não tem testes nomeados + fence.
- **Problem:** TCP Montanha é cleartext; RFC-0021 P2.3 é só doc.
- **Problem:** `property_int_value` devolve sempre `None`; ingest/`delete_files_in_range` não recusam em código.
- **Problem:** default WAL PIT vs fail-stop ainda é do dono — falta o lock mecânico contra skip-any.

## Proposed Solution

- Scoreboard vivo + RFC com paredes explícitas.
- Kernel: fence em I/O de flush/compact explícitos; testes ENOSPC/EIO; compact sob range-delete termina.
- io_uring: wrap FailingEnv + inject CQE negativo.
- Compat: `NotSupported` honesto + properties mapeadas a `DbStats`.
- Store: feature `tls` (rustls), `--require-tls`, health default loopback.
- WAL: exactamente dois modos; skip-any não compila. Default do dono (RFC-0038 P1.1) continua parked.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)

- [x] **P0.1** Scoreboard + lock `WalRecovery` (2 modos; skip-any ausente) + gap #9 docs — status: `done`
- [x] **P0.2** io_uring: FailingEnv wrap + CQE `res<0` (EIO/ENOSPC) sem false-Ok — status: `done`
- [x] **P0.3** ENOSPC/EIO mid-flush / mid-compact / mid-MANIFEST + range-delete compact termina — status: `done`
- [x] **P0.4** Contrato do teto de escrita (group commit; apply no lock) — status: `done` (P2.1 moved apply to the second write-lock hold, after durable fd; still serialized — not a skiplist)
- [x] **P0.5** TLS 1.3 lab no MTCP (`--features tls`, `--require-tls`, health localhost) — status: `done`
- [x] **P0.6** rust-rocksdb 0.22 API (ingest/`SstFileWriter`, `delete_file_in_range` via tombstones, WBWI, compaction filter, CF lifecycle, properties) — status: `done`
- [x] **P0.7** WAL: kernel FailClosed, compat PIT, sem terceiro modo silencioso — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)

- [x] **P1.1** Iterator lazy no compat (`StreamingVisibleIter`) — status: `done` (`DBIterator` + `ITER_WINDOW=64` / `page_forward`; RFC-0032 P0.1 — never materialises the whole CF)
- [ ] **P1.2** `WalRecovery::Evacuate` (B2) só se o dono escolher RFC-0038 P1.1 — status: `todo` (0038 P1.1 parked)
- [x] **P1.3** Rotação de certs + health HTTP TLS — status: `done` (`reload_from_pem_files` replaces process TLS; health HTTP uses `maybe_server_wrap`; tests `tls_reload_from_pem_files_replaces_config` + `/ready` over TLS in `tcp_tls_mtls_roundtrip`)
- [x] **P1.4** `WriteBatchWithIndex` mínimo (read-your-writes, sem Merge) — status: `done` (`WriteBatchWithIndex` + `wbwi_read_your_writes`)

### P2 — later / polish

- [x] **P2.1** RFC-0045 memtable apply fora do lock (teto +15%, não 5×) — status: `done` (`finish_group_off_lock` apply after durable fd; still serialized; not Rocks skiplist — RFC-0055 P1.1 gated)
- [x] **P2.2** det_io CONTRACT-OK + QEMU guest (runner Linux + imagem) — status: `done` (RFC-0052 P2.1–P2.3: `tcg_world_smoke.sh` + `tcg_world_smoke_detio.sh`; CI `tcg-world-smoke` / `tcg-world-smoke-detio`)
- [x] **P2.3** dm-error / page-cache EIO de bloco — status: `done` (`scripts/tcg_blk_eio.sh`: virtio-blk + QEMU blkdebug `flush_to_disk` errno=5; World fail-closed `os error 5`; `silent_wrong` not printed because run aborted; CI `tcg-blk-eio`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | scoreboard + WAL enum lock + gap #9 | done | this change | 2026-08-23 |
| P0.2 | p0 | io_uring wrap + CQE inject | done | this change | 2026-08-23 |
| P0.3 | p0 | ENOSPC/EIO flush/compact/MANIFEST | done | this change | 2026-08-23 |
| P0.4 | p0 | write-group ceiling contract | done | this change | 2026-08-23 |
| P0.5 | p0 | MTCP TLS lab + montanha-secure | done | this change | 2026-08-23 |
| P0.6 | p0 | runbook + NotSupported + properties | done | this change | 2026-08-23 |
| P0.7 | p0 | WAL two-mode lock; 0038 P1.1 parked | done | this change | 2026-08-23 |
| P1.1 | p1 | lazy iterator | done | `DBIterator` window 64 | 2026-08-24 |
| P1.2 | p1 | Evacuate iff owner | todo | RFC-0038 P1.1 parked | 2026-08-23 |
| P1.3 | p1 | cert rotation + health TLS | done | `reload_from_pem_files` + health wrap | 2026-08-24 |
| P1.4 | p1 | WBWI mínimo | done | `wbwi_read_your_writes` | 2026-08-24 |
| P2.1 | p2 | concurrent memtable apply | done | apply after fd (serialized 2nd hold); 0055 P1.1 still gated | 2026-08-24 |
| P2.2 | p2 | det_io CONTRACT-OK + QEMU | done | RFC-0052 P2.1–P2.3 | 2026-08-24 |
| P2.3 | p2 | block EIO | done | `tcg_blk_eio.sh` + job `tcg-blk-eio` | 2026-08-24 |

## Acceptance Criteria

- **Tests:** `wal_recovery_exactly_two_modes`; `linux_cqe_eio_is_not_ok`; `enospc_mid_flush_fences_transient`; `eio_mid_compact_fail_closed`; `enospc_mid_manifest_rename`; `range_delete_compact_terminates`; `write_group_amortizes_apply_still_locked`; ingest/`delete_files_in_range` Err; `property_int_value` estimate-num-keys; `tcp_tls_mtls_roundtrip` (feature `tls`); `--require-tls` recusa cleartext.
- **Telemetry / Analytics:** none — lab gates, not a metrics product.
- **Documentation:** this RFC, scoreboard, runbook, usage ConcurrentDb ceiling, 0021-tls, 0038 P1.1 parked.
- **Screenshots:** none — backend-only.

## Out of scope

Campo real; QEMU guest; memtable Rocks; skip-any; Evacuate default; ingest/filters/WBWI/CFs reais; encrypt-at-rest no engine; claim de GA / TiKV drop-in / field peer.
