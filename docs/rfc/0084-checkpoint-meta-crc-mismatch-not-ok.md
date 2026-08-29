# RFC: 0084 — Checkpoint meta CRC mismatch is not Ok (never load flipped meta)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0083](0083-wal-crc-open-fail-closed.md), [0060](0060-field-and-hardware-residuals.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk** — Sim2’s disk is a model. Pedra’s `CHECKPOINT` file (`PDBCKP02`) carries a trailing CRC32C. WAL/SST/vlog/MANIFEST already call `crc_match_ok`. `read_checkpoint_meta` still compared `stored != computed` inline. AS-IS treats any checksum as matching (flipped last_sequence/sst_count served as meta). This slice names the gate on the **live checkpoint read path**: mismatch is Err containing `crc mismatch`. The lie is **only the trailer** (payload intact) so dropping the gate would still parse.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- `Db::create_checkpoint` writes `CHECKPOINT` via `write_checkpoint_meta` (CRC trailer).
- `read_checkpoint_meta` is the production reader (restore / verify).
- RFC-0083 P1.1 named this remaining site. A mid-file flip is not the CRC tooth (magic/seq parse can fail independently).

## Problems This Solves

- **Problem:** checkpoint CRC lived in glue next to the reader.
- **Problem:** AS-IS would return Ok meta after a trailer lie.
- **Problem:** WAL/SST/vlog/MANIFEST used the named gate; checkpoint did not.

## Proposed Solution

- Production `read_checkpoint_meta` calls `crc_match_ok`. No new `*_kernel.rs`. Do not extract `db.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live checkpoint reader)
- [x] **P0.1** `read_checkpoint_meta` calls `crc_match_ok` — status: `done`
- [x] **P0.2** Regression: live `create_checkpoint`, XOR trailer only, read is crc mismatch — status: `done` (`crc_mismatch_on_live_checkpoint_meta_is_not_ok`)

### P1 — next wave
- [x] **P1.1** Changelog decode uses `crc_match_ok` (open still fail-open F33) — status: `done` (RFC-0085)
- [x] **P1.2** `verify_at_rest` CHECKPOINT walk already uses the gate; keep it — status: `done` (`crc_mismatch_on_live_checkpoint_verify_at_rest_is_not_ok`)

### P2 — later
- [x] **P2.1** Collision axiom remains R-crc — status: `done` (`checkpoint_crc_collision_axiom_remains`)
- [x] **P2.2** Backup CATALOG CRC stays RFC-0060 — status: `done` (`backup_catalog_crc_stays_rfc0060`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | read_checkpoint_meta gated | done | db.rs | 2026-08-27 |
| P0.2 | p0 | trailer lie is crc mismatch | done | crc_mismatch_on_live_checkpoint_meta_is_not_ok | 2026-08-27 |
| P1.1 | p1 | changelog CRC | done | RFC-0085 | 2026-08-27 |
| P1.2 | p1 | verify_at_rest CHECKPOINT | done | crc_mismatch_on_live_checkpoint_verify_at_rest_is_not_ok | 2026-08-28 |
| P2.1 | p2 | R-crc collision | done | checkpoint_crc_collision_axiom_remains | 2026-08-28 |
| P2.2 | p2 | backup CATALOG | done | backup_catalog_crc_stays_rfc0060 | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_checkpoint_meta_is_not_ok`: production `Db::create_checkpoint`, XOR only the last 4 CRC bytes of `CHECKPOINT` (payload intact). `read_checkpoint_meta` is Err containing `crc mismatch`. Dropping `crc_match_ok` (AS-IS) would return Ok meta. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.2 `crc_mismatch_on_live_checkpoint_verify_at_rest_is_not_ok`: same trailer lie; `verify_at_rest` is not clean and names `CHECKPOINT` `crc mismatch`. Walk stays on `read_checkpoint_meta`. Catalog `crc_match` caller `verify.rs`.
  - P2.1 `checkpoint_crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `R-crc` stays in `never_floor`.
  - P2.2 `backup_catalog_crc_stays_rfc0060`: CATALOG trailer lie is named by `verify_at_rest` via RFC-0060 `check_magic_crc_trailer`; product live tooth remains ops `crc_mismatch_on_live_ops_catalog_is_not_ok`. This RFC does not take CATALOG ownership.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0084; RFC-0083 P1.1 marked done.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- ECC / CPU proof. CRC collision (R-crc). Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Rocks coluna A/B. crates.io.
