# RFC: 0081 — Vlog CRC mismatch is not Ok (never serve flipped payload)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0060](0060-field-and-hardware-residuals.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 field/media** — Sim2’s disk is a model. Pedra’s value log (`VALUES.vlog`) stores CRC32C per record. WAL (0076) and SST files (0077) already call `crc_match_ok`. The vlog still compared `stored_crc != expect_crc` inline in `read_pending` / `read_record_at`. AS-IS treats any checksum as matching (flipped blob served as a value). This slice names the gate on the **live vlog read path**: mismatch is `CorruptValue`, never Ok.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. This is not an ECC proof. Bits after the last scrub stay residual. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- RFC-0076: WAL `crc_match_ok`. RFC-0077: SST file CRC fate.
- Vlog layout: `len | crc | data`. Pointers carry the same CRC. Read checks header CRC and data CRC.
- FDB Sim2 does not CRC-walk a real value log. Pedra must not round a mismatch to a live blob.

## Problems This Solves

- **Problem:** vlog CRC compares were glue; a drift could serve corruption as a value.
- **Problem:** AS-IS always matches (silent-wrong blob).
- **Problem:** WAL/SST used the named gate; vlog did not.

## Proposed Solution

- Production `read_pending` and `read_record_at` call `crc_match_ok`. No new `*_kernel.rs`. Do not extract `db.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (gate on the live vlog reader)
- [x] **P0.1** `read_pending` + `read_record_at` call `crc_match_ok` — status: `done`
- [x] **P0.2** Regression: live append, flip payload, read is CorruptValue — status: `done` (`crc_mismatch_on_live_vlog_is_not_ok`)

### P1 — next wave
- [x] **P1.1** Blob-file (`*.blob`) reads use the same gate — status: `done` (`crc_mismatch_on_live_blob_is_not_ok`)
- [x] **P1.2** `Db::get` of a flipped large value is Err, not a flipped blob — status: `done` (`crc_mismatch_on_live_db_get_large_value_is_not_ok`)

### P2 — later
- [x] **P2.1** Catalog / Verus token — status: `done` (catalog `crc_match` caller `vlog.rs`; twin `crc_match.rs`)
- [x] **P2.2** Collision axiom remains R-crc — status: `done` (`vlog_crc_collision_axiom_remains`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | vlog reads gated | done | vlog.rs | 2026-08-27 |
| P0.2 | p0 | flipped vlog is CorruptValue | done | crc_mismatch_on_live_vlog_is_not_ok | 2026-08-27 |
| P1.1 | p1 | blob file CRC | done | crc_mismatch_on_live_blob_is_not_ok | 2026-08-28 |
| P1.2 | p1 | Db::get flipped blob | done | crc_mismatch_on_live_db_get_large_value_is_not_ok | 2026-08-28 |
| P2.1 | p2 | catalog + Verus | done | crc_match callers += vlog.rs | 2026-08-28 |
| P2.2 | p2 | R-crc collision | done | vlog_crc_collision_axiom_remains | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_vlog_is_not_ok`: production `ValueLog::append` (real file, StdEnv), flip a payload byte, `read_at_on` is `CorruptValue`. Runs on Darwin.
  - P1.1 `crc_mismatch_on_live_blob_is_not_ok`: `ValueLog::open_blob` + `append` to `000001.blob`, flip a payload byte, `read_ptr_on` (`file_num=1`) is `CorruptValue`. AS-IS would serve the flipped blob. Same `crc_match_ok` as `VALUES.vlog`. No new `*_kernel.rs`.
  - P1.2 `crc_mismatch_on_live_db_get_large_value_is_not_ok`: `Db::open` with `large_value_threshold`, put ≥ threshold, close, flip `VALUES.vlog` payload. Reopen `get_at` is Err containing `crc` (does not serve the flipped bytes). `get` fail-stops via the same `resolve_stored_value` (F1). AS-IS would serve the blob.
  - P2.1 catalog pair `crc_match` lists `vlog.rs` as a caller of `crc_match_ok` (same Verus twin as RFC-0076; freeze of twin files; `verus` not on PATH). No new `*_kernel.rs`.
  - P2.2 `vlog_crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `residuals.json` still lists `R-crc` in `never_floor`. Equality of two u32s is not a collision theorem.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0081.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- ECC / CPU proof. CRC collision (R-crc). Proving the drive (R-fsync-lie).
- Restoring production WAL onto io_uring. Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Rocks coluna A/B. crates.io.
