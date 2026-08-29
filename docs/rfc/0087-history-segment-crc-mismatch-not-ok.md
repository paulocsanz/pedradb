# RFC: 0087 — History segment record CRC mismatch is not Ok (never walk flipped versions)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0086](0086-history-manifest-crc-mismatch-not-ok.md), [0046](0046-mvcc-history-tiering-s3.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk** — Sim2’s disk is a model. Pedra’s `history/seg-*.hist` records carry a per-record CRC32C (`klen|key|vlen|val|seq|kind` then CRC). `walk_segment_records` still compared `crc32c(prefix) != stored` inline. AS-IS treats any checksum as matching (flipped archived version served at restore/upload). This slice names the gate on the **live segment walk**: mismatch is `CorruptHistory` containing `crc mismatch`. The lie is **only the last record’s CRC trailer** (key/len/seq intact).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- `HistoryTier::archive_stream` appends CRC’d records then fsyncs the segment.
- `walk_segment_records` is the production walker (upload + restore).
- RFC-0086 P1.1 named this remaining site. A key/len flip is not the CRC tooth (truncated-header can fail independently).

## Problems This Solves

- **Problem:** segment CRC lived in glue next to the walker.
- **Problem:** AS-IS would return the archived versions after a trailer lie.
- **Problem:** history MANIFEST used the named gate; per-record `.hist` did not.

## Proposed Solution

- Production `walk_segment_records` calls `crc_match_ok`. No new `*_kernel.rs`. Do not extract `db.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live segment walk)
- [x] **P0.1** `walk_segment_records` calls `crc_match_ok` — status: `done`
- [x] **P0.2** Regression: live `archive_stream`, XOR last-record CRC only, walk is crc mismatch — status: `done` (`crc_mismatch_on_live_history_segment_is_not_ok`)

### P1 — next wave
- [x] **P1.1** Bloom sidecar CRC uses `crc_match_ok` — status: `done` (RFC-0088)
- [x] **P1.2** Remote `LATEST` pointer CRC uses `crc_match_ok` — status: `done` (RFC-0089)

### P2 — later
- [x] **P2.1** Collision axiom remains R-crc — status: `done` (`history_segment_crc_collision_axiom_remains`)
- [x] **P2.2** Upload/restore callers stay on `walk_segment_records` — status: `done` (`history_segment_upload_restore_stay_on_walk`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | walk_segment_records gated | done | history.rs | 2026-08-27 |
| P0.2 | p0 | last-record CRC lie | done | crc_mismatch_on_live_history_segment_is_not_ok | 2026-08-27 |
| P1.1 | p1 | bloom sidecar CRC | done | RFC-0088 | 2026-08-27 |
| P1.2 | p1 | remote LATEST CRC | done | RFC-0089 | 2026-08-27 |
| P2.1 | p2 | R-crc collision | done | history_segment_crc_collision_axiom_remains | 2026-08-28 |
| P2.2 | p2 | callers stay on walk | done | history_segment_upload_restore_stay_on_walk | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_history_segment_is_not_ok`: production `HistoryTier::archive_stream` (real `seg-*.hist`). XOR only the last 4 bytes of the file (last record CRC; key/len intact). `walk_segment_records` is Err containing `crc mismatch`. Dropping `crc_match_ok` (AS-IS) would return the three archived versions. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 `history_segment_crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `R-crc` stays in `never_floor`.
  - P2.2 `history_segment_upload_restore_stay_on_walk`: production callers (`put_segment`, `db.rs` restore, `verify.rs` scrub, ops `restore_history_from_remote`) stay on `walk_segment_records`. Same last-record CRC lie: `put_segment` is crc mismatch and uploads nothing; `verify_at_rest` names `history/seg-*.hist` crc mismatch. Payload flip (`remote_segment_put_refuses_corrupt_local`) is not this tooth.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0087; RFC-0086 P1.1 marked done.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- ECC / CPU proof. CRC collision (R-crc). Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Rocks coluna A/B. crates.io.
