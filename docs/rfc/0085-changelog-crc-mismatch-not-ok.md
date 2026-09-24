# RFC: 0085 — CHANGELOG CRC mismatch is not Ok (decode fail-closed; open stays F33)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0084](0084-checkpoint-meta-crc-mismatch-not-ok.md), [0060](0060-field-and-hardware-residuals.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk** — Sim2’s disk is a model. Pedra’s `CHANGELOG` cache carries a trailing CRC32C. WAL/SST/vlog/MANIFEST/CHECKPOINT already call `crc_match_ok`. `decode_changelog` still compared `stored != got` inline. AS-IS treats any checksum as matching (flipped feed served as cache). This slice names the gate on the **live decode path** (`pedra verify` / `ChangeLog::load_on`): mismatch is Err containing `crc mismatch`. The lie is **only the trailer** (payload intact). `Db::open` still fail-opens (F33: cache, WAL rebuild).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring). F33 is not undone.

## Background

- `close` / flush persist the CHANGELOG cache (`encode_changelog` + fsync).
- `decode_changelog` is the production decoder (`load_on`, `verify_at_rest`).
- Existing `corrupt_changelog_does_not_block_open` XORs a payload byte (offset 12), which can fail parse independently of the CRC gate.
- RFC-0084 P1.1 named this remaining site.

## Problems This Solves

- **Problem:** changelog CRC lived in glue next to the decoder.
- **Problem:** AS-IS would decode Ok after a trailer lie.
- **Problem:** a payload flip is not a CRC-gate tooth.

## Proposed Solution

- Production `decode_changelog` calls `crc_match_ok`. No new `*_kernel.rs`. Do not extract `db.rs`. Open remains F33.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live CHANGELOG decode)
- [x] **P0.1** `decode_changelog` calls `crc_match_ok` — status: `done`
- [x] **P0.2** Regression: live `put`+`close`, XOR trailer only, decode is crc mismatch; `Db::open` still Ok — status: `done` (`crc_mismatch_on_live_changelog_is_not_ok`)

### P1 — next wave
- [x] **P1.1** `verify_at_rest` names CHANGELOG on the same trailer lie — status: `done` (`crc_mismatch_on_live_changelog_verify_at_rest_is_not_ok`)
- [x] **P1.2** History MANIFEST CRC uses `crc_match_ok` — status: `done` (RFC-0086)

### P2 — later
- [x] **P2.1** Collision axiom remains R-crc — status: `done` (`changelog_crc_collision_axiom_remains`)
- [x] **P2.2** F33 quarantine stays fail-open on `Db::open` — status: `done` (`changelog_crc_mismatch_open_still_quarantines_f33`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | decode_changelog gated | done | change_feed.rs | 2026-08-27 |
| P0.2 | p0 | trailer lie is crc mismatch | done | crc_mismatch_on_live_changelog_is_not_ok | 2026-08-27 |
| P1.1 | p1 | verify_at_rest names it | done | crc_mismatch_on_live_changelog_verify_at_rest_is_not_ok | 2026-08-28 |
| P1.2 | p1 | history MANIFEST | done | RFC-0086 | 2026-08-27 |
| P2.1 | p2 | R-crc collision | done | changelog_crc_collision_axiom_remains | 2026-08-28 |
| P2.2 | p2 | F33 open stays | done | changelog_crc_mismatch_open_still_quarantines_f33 | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_changelog_is_not_ok`: production `Db::open` + `put` + `close` (CHANGELOG on disk). XOR only the last 4 CRC bytes (payload intact). `decode_changelog` is Err containing `crc mismatch`. `Db::open` still succeeds (F33). Dropping `crc_match_ok` (AS-IS) would decode Ok. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `crc_mismatch_on_live_changelog_verify_at_rest_is_not_ok`: same trailer lie; `verify_at_rest` is not clean and names `CHANGELOG` `crc mismatch`. Walk stays on `decode_changelog`. F33 open still Ok.
  - P2.1 `changelog_crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `R-crc` stays in `never_floor`.
  - P2.2 `changelog_crc_mismatch_open_still_quarantines_f33`: same trailer lie; `Db::open` is Ok; poison is renamed to `CHANGELOG.corrupt`; WAL rebuild serves k. F33 is not undone.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0085; RFC-0084 P1.1 marked done.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Making CHANGELOG source of truth (WAL remains). Undoing F33.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- ECC / CPU proof. CRC collision (R-crc). Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Rocks coluna A/B. crates.io.
