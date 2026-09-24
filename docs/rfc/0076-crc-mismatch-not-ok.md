# RFC: 0076 — CRC mismatch is not Ok (never serve as miss)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0060](0060-field-and-hardware-residuals.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 field/media** — Sim2’s disk is a model (`AsyncFileNonDurable`); Pedra’s WAL/SST/vlog carry CRC32C and fail-closed on mismatch. The compare `stored == computed` lived inline in the WAL reader and the at-rest scrub. AS-IS treats any checksum as matching (corruption served as a valid record / clean scrub). This slice names the gate on the **live recover path**: mismatch is `CoreError::Crc`, never Ok.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still an axiom (R-crc). This is not an ECC proof. Bits after the last scrub stay R-hardware.

## Background

- RFC-0060: `verify_at_rest` + BitFlip. Read path already fail-closed; the predicate was glue.
- WAL `WalReader` compares stored vs `record_checksum` and returns `Crc`.
- FDB Sim2 does not CRC-walk a real WAL. Pedra must not round a mismatch to torn-tail / miss.

## Problems This Solves

- **Problem:** `stored_crc != actual_crc` was inline; a drift could swallow corruption as EOF.
- **Problem:** AS-IS always matches (silent-wrong record).
- **Problem:** scrub and WAL recover duplicated the compare.

## Proposed Solution

- Pure `crc_match_ok(stored, computed)` = equality. AS-IS always true.
- Production `WalReader` and `verify_at_rest` (vlog / CURRENT / magic trailer) call it. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (gate on the live WAL reader)
- [x] **P0.1** `crc_match_ok` + AS-IS in `wal/crc.rs` — status: `done`
- [x] **P0.2** `WalReader` + verify CRC sites call the gate — status: `done`
- [x] **P0.3** Regression: live write, flip payload, read is `Crc`; AS-IS would accept — status: `done` (`crc_mismatch_on_live_wal_is_not_ok`)

### P1 — next wave
- [x] **P1.1** SST file-trailer CRC uses the same gate — status: `done` (RFC-0077 P0 `sst_crc_fate`; `sst_file_crc_uses_crc_match_ok`)
- [x] **P1.2** `Db::open` of a WAL CRC-field lie is Err (`Crc`) — status: `done` (RFC-0083 `crc_mismatch_on_live_wal_open_is_not_ok`)

### P2 — later
- [x] **P2.1** Catalog pair + Verus twin — status: `done` (`verus/crc_match.rs` + catalog `crc_match`)
- [x] **P2.2** Collision axiom remains R-crc — status: `done` (`crc_collision_admitted`; `crc_collision_axiom_remains`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | crc_match_ok + AS-IS | done | wal/crc.rs | 2026-08-27 |
| P0.2 | p0 | WalReader + verify gated | done | reader.rs + verify.rs | 2026-08-27 |
| P0.3 | p0 | flipped WAL is Crc | done | crc_mismatch_on_live_wal_is_not_ok | 2026-08-27 |
| P1.1 | p1 | SST file-trailer CRC | done | sst_file_crc_uses_crc_match_ok (0077 P0) | 2026-08-28 |
| P1.2 | p1 | Db::open WAL CRC-field lie | done | RFC-0083 crc_mismatch_on_live_wal_open_is_not_ok | 2026-08-27 |
| P2.1 | p2 | catalog + Verus | done | crc_match.rs + catalog crc_match | 2026-08-28 |
| P2.2 | p2 | R-crc collision | done | crc_collision_admitted | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_wal_is_not_ok`: `WalWriter` writes a record (production writer), flip a payload byte, `WalReader::read_record` is `CoreError::Crc`. Runs on Darwin.
  - P1.1 `sst_file_crc_uses_crc_match_ok`: `sst_crc_fate` mismatch on a modern file is `Reject`; match is `StripTrailer`; AS-IS strips. File trailer is RFC-0077 P0 (`crc_mismatch_on_live_sst_is_not_ok`). Per-block SST CRC remains RFC-0077 P1.1.
  - P2.1 catalog pair `crc_match` entry `crc_match_ok` with Verus twin (freeze of twin files; `verus` not on PATH). No new `*_kernel.rs`.
  - P2.2 `crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `residuals.json` still lists `R-crc` in `never_floor`. Equality of two u32s is not a collision theorem.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0076.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- ECC / CPU proof. CRC collision (R-crc). Proving the drive (R-fsync-lie).
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`. Rocks coluna A/B. crates.io.
