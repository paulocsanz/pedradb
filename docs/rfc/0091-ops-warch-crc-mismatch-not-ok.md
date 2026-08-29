# RFC: 0091 — Ops WAL-archive CRC mismatch is not Ok (never restore a flipped increment)

**Status:** done
**Updated:** 2026-08-27
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0090](0090-raft-hard-crc-mismatch-not-ok.md), [0014](0014-backup-pitr.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk / G8 recover** — Sim2’s disk is a model. Pedra’s `wal/*.warch` (`PDBWAR01` + count + records + trailing CRC32C) is the PITR increment. Catalog (0090 P2.1) already calls `crc_match_ok`. `read_warch` still compared `crc32c(payload) != stored` inline. AS-IS treats any checksum as matching (flipped WAL increment replayed on restore). This slice names the gate on the **live ship/verify path**: mismatch is Err containing `crc mismatch`. The lie is **only the trailer** (magic/count/records intact). `verify_backup_flags_corrupt_warch` XORs the last byte but does not pin `crc_match_ok` vs AS-IS.

P1.1 gates CURRENT classify. P1.2 pins restore (`restore_with_increments` → `read_warch`) on the same trailer lie. P2.1 gates bloom prune with `crc_match_ok` but **fail-open** (walk, never skip). P2.2 records that CRC32C collision **remains** `R-crc` in `never_floor`.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- `BackupEngine::ship_wal` / `create_incremental` writes `wal/NNNNNN.warch` then fsyncs.
- `verify_wal_archive` / `restore_pitr` call `read_warch`. CRC is checked **before** magic.
- RFC-0090 P2.1 gated `CATALOG`; warch stayed inline.

## Problems This Solves

- **Problem:** warch CRC lived in ops glue next to the record walker.
- **Problem:** AS-IS would decode the intact records after a trailer lie.
- **Problem:** CATALOG used the named gate; the increment file did not.
- **Problem:** CURRENT classify and bloom prune still compared inline.

## Proposed Solution

- Production `read_warch`, `classify_current_crc`, and bloom prune (`sidecar_may_affect`) call `crc_match_ok`. Restore stays on `read_warch`. Prune stays fail-open. No new `*_kernel.rs`. Do not extract `db.rs`. Collision axiom stays `R-crc`.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live warch read)
- [x] **P0.1** `read_warch` calls `crc_match_ok` — status: `done`
- [x] **P0.2** Regression: live `create_incremental`, XOR trailer only, verify is crc mismatch — status: `done` (`crc_mismatch_on_live_ops_warch_is_not_ok`)

### P1 — next wave
- [x] **P1.1** `classify_current_crc` uses `crc_match_ok` — status: `done` (`crc_mismatch_on_live_ops_current_classify_is_not_ok`)
- [x] **P1.2** Restore path stays on `read_warch` — status: `done` (`crc_mismatch_on_live_ops_warch_restore_is_not_ok`)

### P2 — later
- [x] **P2.1** Read-path bloom prune uses `crc_match_ok` (still fail-open: walk on mismatch) — status: `done` (`crc_mismatch_on_live_history_bloom_prune_still_walks`)
- [x] **P2.2** Collision axiom remains R-crc — status: `done` (`never_floor` still lists `R-crc`; not a proof)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | read_warch gated | done | ops lib.rs read_warch | 2026-08-27 |
| P0.2 | p0 | trailer lie is crc mismatch | done | crc_mismatch_on_live_ops_warch_is_not_ok | 2026-08-27 |
| P1.1 | p1 | CURRENT classify uses crc_match_ok | done | crc_mismatch_on_live_ops_current_classify_is_not_ok | 2026-08-27 |
| P1.2 | p1 | restore stays on read_warch | done | crc_mismatch_on_live_ops_warch_restore_is_not_ok | 2026-08-27 |
| P2.1 | p2 | fail-open bloom prune | done | crc_mismatch_on_live_history_bloom_prune_still_walks | 2026-08-27 |
| P2.2 | p2 | R-crc collision | done | never_floor still lists R-crc | 2026-08-27 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_ops_warch_is_not_ok`: production `BackupEngine::create_incremental` (real `wal/*.warch`, StdEnv). XOR only the last 4 CRC bytes (magic/count/records intact). `verify_wal_archive` is Err containing `crc mismatch`. Dropping `crc_match_ok` (AS-IS) would return the shipped records. Runs on Darwin. Does not submit io_uring SQEs.
  - `crc_mismatch_on_live_ops_current_classify_is_not_ok`: production `inspect_format_env` is `ok` on a live CURRENT, then XOR stored CRC u32, rewrite 8 hex (name/MANIFEST intact). `classify_current_crc` is `mismatch` (`inspect_format_env` fail-closes in `manifest::load` before classify). AS-IS would report `ok`. `inspect_classifies_current_crc` (`ffffffff`) is **not** this tooth.
  - `crc_mismatch_on_live_ops_warch_restore_is_not_ok`: production `restore_with_increments` (still `read_warch`). XOR trailer only. Restore is crc mismatch. Verify-path tests are **not** this tooth.
  - `crc_mismatch_on_live_history_bloom_prune_still_walks`: production `segment_may_affect`. XOR sidecar trailer only. Intact bloom prunes `zzz`; after the lie, prune still walks. AS-IS would prune. Fail-closed `verify_bloom_sidecar` is RFC-0088.
  - Existing `verify_backup_flags_corrupt_warch` is **not** the P0 tooth.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text names 0091 P0–P2; `R-crc` stays `never` in `never_floor`.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- ECC / CPU proof. CRC collision (R-crc remains `never_floor`). Extracting `db.rs`. New `*_kernel.rs`.
- Making the history bloom prune fail-closed.
- Rocks coluna A/B. crates.io.
