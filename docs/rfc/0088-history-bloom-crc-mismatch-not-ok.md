# RFC: 0088 — History bloom sidecar CRC mismatch is not Ok (never scrub a flipped filter as clean)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0086](0086-history-manifest-crc-mismatch-not-ok.md), [0087](0087-history-segment-crc-mismatch-not-ok.md), [0046](0046-mvcc-history-tiering-s3.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk** — Sim2’s disk is a model. Pedra’s `history/seg-*.bloom` (`PHB1` + body + `body_len` + CRC32C) is the P2.6 prune sidecar. WAL/SST/vlog/MANIFEST/CHECKPOINT/CHANGELOG/history MANIFEST/segment already call `crc_match_ok`. `verify_bloom_sidecar` (at-rest scrub + remote verify) still compared `crc32c(body) != crc` inline and folded CRC failure into a generic `"bloom sidecar"` error. AS-IS treats any checksum as matching (flipped filter reported clean). This slice names the gate on the **live scrub path**: mismatch is `CorruptHistory` containing `crc mismatch`. The lie is **only the trailer** (magic/version/`body_len`/bits intact).

The read-path prune (`sidecar_may_affect`) stays **fail-open**: a damaged sidecar must walk, never skip. That is not this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- `HistoryTier::archive_stream` writes and fsyncs `seg-{id}.bloom` before the MANIFEST lists the segment.
- `verify_at_rest` and `RemoteTier::verify` call `verify_bloom_sidecar`.
- RFC-0086 P1.2 / RFC-0087 P1.1 named this remaining site. A mid-file / `body_len` flip is not the CRC tooth (magic/length fail independently). Generic `"bloom sidecar"` is not a CRC tooth.

## Problems This Solves

- **Problem:** bloom sidecar CRC lived in glue and hid behind a generic error.
- **Problem:** AS-IS would report the sidecar clean after a trailer lie.
- **Problem:** history MANIFEST and `.hist` records used the named gate; the filter sidecar did not.

## Proposed Solution

- Production `verify_bloom_sidecar` calls `crc_match_ok` and names `crc mismatch`. No new `*_kernel.rs`. Do not extract `db.rs`. Do not change fail-open prune.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live sidecar scrub)
- [x] **P0.1** `verify_bloom_sidecar` calls `crc_match_ok` and names crc mismatch — status: `done`
- [x] **P0.2** Regression: live `archive_stream`, XOR trailer only, verify is crc mismatch — status: `done` (`crc_mismatch_on_live_history_bloom_is_not_ok`)

### P1 — next wave
- [x] **P1.1** Read-path `sidecar_may_affect` uses `crc_match_ok` (still fail-open: walk on mismatch) — status: `done` (`crc_mismatch_on_live_history_bloom_sidecar_may_affect_still_walks`)
- [x] **P1.2** Remote `LATEST` pointer CRC uses `crc_match_ok` — status: `done` (RFC-0089)

### P2 — later
- [x] **P2.1** Collision axiom remains R-crc — status: `done` (`history_bloom_crc_collision_axiom_remains`)
- [x] **P2.2** Fail-open prune stays fail-open — status: `done` (`history_bloom_crc_mismatch_prune_stays_fail_open`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | verify_bloom_sidecar gated + named crc | done | history.rs verify_bloom_sidecar | 2026-08-27 |
| P0.2 | p0 | trailer lie is crc mismatch | done | crc_mismatch_on_live_history_bloom_is_not_ok | 2026-08-27 |
| P1.1 | p1 | fail-open prune uses crc_match_ok | done | crc_mismatch_on_live_history_bloom_sidecar_may_affect_still_walks | 2026-08-28 |
| P1.2 | p1 | remote LATEST CRC | done | RFC-0089 | 2026-08-27 |
| P2.1 | p2 | R-crc collision | done | history_bloom_crc_collision_axiom_remains | 2026-08-28 |
| P2.2 | p2 | prune stays fail-open | done | history_bloom_crc_mismatch_prune_stays_fail_open | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_history_bloom_is_not_ok`: production `HistoryTier::archive_stream` (real `seg-*.bloom`). XOR only the last 4 CRC bytes (magic/`body_len`/bits intact). `verify_bloom_sidecar` is Err containing `crc mismatch`. Dropping `crc_match_ok` (AS-IS) would return Ok. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `crc_mismatch_on_live_history_bloom_sidecar_may_affect_still_walks`: same trailer lie on the pure prune parse. Intact bloom prunes `zzz`; after the lie, `sidecar_may_affect` still walks. AS-IS would prune. File-I/O `segment_may_affect` is RFC-0091 P2.1, not this tooth.
  - P2.1 `history_bloom_crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `R-crc` stays in `never_floor`.
  - P2.2 `history_bloom_crc_mismatch_prune_stays_fail_open`: same trailer lie; scrub is crc mismatch; prune still walks; `db.rs` stays on `sidecar_may_affect`. Do not make prune fail-closed.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0088; RFC-0086 P1.2 and RFC-0087 P1.1 marked done.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- ECC / CPU proof. CRC collision (R-crc). Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Making the read-path prune fail-closed (a damaged filter must never skip a segment).
- Rocks coluna A/B. crates.io.
