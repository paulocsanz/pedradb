# RFC: 0086 — History MANIFEST CRC mismatch is not Ok (never load flipped archive inventory)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0085](0085-changelog-crc-mismatch-not-ok.md), [0046](0046-mvcc-history-tiering-s3.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk** — Sim2’s disk is a model. Pedra’s `history/MANIFEST` (`PHST`) carries a trailing CRC32C. WAL/SST/vlog/MANIFEST/CHECKPOINT/CHANGELOG already call `crc_match_ok`. `Manifest::decode` still compared `crc32c(body) != crc` inline and folded CRC failure into a generic `"history manifest"` error. AS-IS treats any checksum as matching (flipped segment list served on `HistoryTier::open`). This slice names the gate on the **live history-tier open path**: mismatch is Err containing `crc mismatch`. The lie is **only the trailer** (payload intact).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- `HistoryTier::archive_stream` seals `history/MANIFEST` after each synced segment.
- `HistoryTier::open` / `verify_history_manifest` decode that file.
- RFC-0085 P1.2 named this remaining site. A mid-file flip is not the CRC tooth (`n` / magic can fail independently). Generic `"history manifest"` is not a CRC tooth.

## Problems This Solves

- **Problem:** history MANIFEST CRC lived in glue and hid behind a generic error.
- **Problem:** AS-IS would decode Ok after a trailer lie.
- **Problem:** LSM MANIFEST used the named gate; the history-tier inventory did not.

## Proposed Solution

- Production `Manifest::decode` calls `crc_match_ok` and names `crc mismatch`. No new `*_kernel.rs`. Do not extract `db.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live HistoryTier open)
- [x] **P0.1** `Manifest::decode` calls `crc_match_ok` and names crc mismatch — status: `done`
- [x] **P0.2** Regression: live `archive_stream`, XOR trailer only, decode/open is crc mismatch — status: `done` (`crc_mismatch_on_live_history_manifest_is_not_ok`)

### P1 — next wave
- [x] **P1.1** History segment record CRC (`walk_segment_records`) uses `crc_match_ok` — status: `done` (RFC-0087)
- [x] **P1.2** Bloom sidecar CRC uses `crc_match_ok` — status: `done` (RFC-0088)

### P2 — later
- [x] **P2.1** Remote `LATEST` pointer CRC uses `crc_match_ok` — status: `done` (RFC-0089)
- [x] **P2.2** Collision axiom remains R-crc — status: `done` (`history_manifest_crc_collision_axiom_remains`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | decode gated + named crc | done | history.rs Manifest::decode | 2026-08-27 |
| P0.2 | p0 | trailer lie is crc mismatch | done | crc_mismatch_on_live_history_manifest_is_not_ok | 2026-08-27 |
| P1.1 | p1 | segment record CRC | done | RFC-0087 | 2026-08-27 |
| P1.2 | p1 | bloom sidecar CRC | done | RFC-0088 | 2026-08-27 |
| P2.1 | p2 | remote LATEST CRC | done | RFC-0089 | 2026-08-27 |
| P2.2 | p2 | R-crc collision | done | history_manifest_crc_collision_axiom_remains | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_history_manifest_is_not_ok`: production `HistoryTier::archive_stream` (real `history/MANIFEST`). XOR only the last 4 CRC bytes (payload intact). `verify_history_manifest` is Err containing `crc mismatch`. `HistoryTier::open` is Err. Dropping `crc_match_ok` (AS-IS) would decode Ok. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.2 `history_manifest_crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `R-crc` stays in `never_floor`. Not an ECC / collision theorem.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0086; RFC-0085 P1.2 marked done.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- ECC / CPU proof. CRC collision (R-crc). Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Rocks coluna A/B. crates.io.
