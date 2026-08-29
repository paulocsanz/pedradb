# RFC: 0083 — `Db::open` of a WAL CRC-field lie is refused (FailClosed)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0038](0038-wal-corruption-recovery-open-decision.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk / G8 recover** — Sim2’s disk is a model. Pedra’s WAL reader already fail-closes on `crc_match_ok` (RFC-0076). The missing tooth was the **live `Db::open` recover path**: a CRC-field lie with the payload intact must refuse open under default FailClosed. AS-IS would replay the intact payload and serve the key. Flipping a payload byte is not this tooth (decode of the write record can fail for other reasons).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- RFC-0076 P0: `WalReader` → `CoreError::Crc`. P1.2 (`Db::open` of a flipped WAL) was still todo.
- Default `WalRecovery::FailClosed`: `reopen_outcome(Crc)` is `RefuseOpen`.
- Existing `wal_crc_corruption_journals_then_escalates_then_recovers` XORs a payload byte (offset 30), which can fail parse independently of the CRC gate.

## Problems This Solves

- **Problem:** WAL CRC was proven on the reader, not named on `Db::open` recover with a parse-preserving lie.
- **Problem:** AS-IS `crc_match_ok` would open and serve the durable key.
- **Problem:** a payload flip is not a CRC-gate tooth.

## Proposed Solution

- Keep `crc_match_ok` on `WalReader` (no new `*_kernel.rs`). Named live test: production `put` (fdatasync before Ok), lie only on the stored CRC bytes, `Db::open` is Err(`Crc`). Do not extract `db.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live Db::open recover)
- [x] **P0.1** Named recover tooth: CRC-field lie, payload intact — status: `done` (`crc_mismatch_on_live_wal_open_is_not_ok`)

### P1 — next wave
- [x] **P1.1** Checkpoint meta CRC uses `crc_match_ok` — status: `done` (RFC-0084)
- [x] **P1.2** Changelog decode uses `crc_match_ok` (open still fail-open F33) — status: `done` (RFC-0085 P0 `crc_mismatch_on_live_changelog_is_not_ok`)

### P2 — later
- [x] **P2.1** Collision axiom remains R-crc — status: `done` (`wal_open_crc_collision_axiom_remains`)
- [x] **P2.2** PointInTime still reports+serves prefix (0047); this RFC does not change that — status: `done` (`wal_crc_field_lie_point_in_time_serves_prefix`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Db::open CRC-field lie refused | done | crc_mismatch_on_live_wal_open_is_not_ok | 2026-08-27 |
| P1.1 | p1 | checkpoint meta CRC | done | RFC-0084 | 2026-08-27 |
| P1.2 | p1 | changelog CRC (F33 fail-open) | done | RFC-0085 crc_mismatch_on_live_changelog_is_not_ok | 2026-08-28 |
| P2.1 | p2 | R-crc collision | done | wal_open_crc_collision_axiom_remains | 2026-08-28 |
| P2.2 | p2 | PointInTime prefix stays | done | wal_crc_field_lie_point_in_time_serves_prefix | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_wal_open_is_not_ok`: `Db::open` + `put` + `close` (production WAL). XOR only a CRC header byte (`CURRENT.log[0]`), payload intact. `Db::open` is `CoreError::Crc`. Dropping `crc_match_ok` (AS-IS) would open and serve the key. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.2 `crc_mismatch_on_live_changelog_is_not_ok` (RFC-0085 P0): CHANGELOG trailer XOR; `decode_changelog` is crc mismatch; `Db::open` still Ok (F33). Catalog `crc_match` caller `change_feed.rs`.
  - P2.1 `wal_open_crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `R-crc` stays in `never_floor`.
  - P2.2 `wal_crc_field_lie_point_in_time_serves_prefix`: two puts; XOR CRC of the *second* record. FailClosed is `Crc`. PointInTime opens, serves `k00`, drops `k01`, `last_recovery_report().kind == "crc"`. `reopen_outcome(Crc, PIT, false)` is `ServePrefixReport`; AS-IS is `ServeAll`. This RFC does not change FailClosed vs PointInTime.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0083; RFC-0076 P1.2 marked done.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- Changing FailClosed vs PointInTime. Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Rocks coluna A/B. crates.io.
