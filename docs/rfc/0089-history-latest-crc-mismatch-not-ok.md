# RFC: 0089 — Remote LATEST pointer CRC mismatch is not Ok (never scrub a flipped generation pointer as clean)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0086](0086-history-manifest-crc-mismatch-not-ok.md), [0088](0088-history-bloom-crc-mismatch-not-ok.md), [0046](0046-mvcc-history-tiering-s3.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk** — Sim2’s disk is a model. Pedra’s remote `LATEST` is `MANIFEST-<n>\n<crc32c hex of that generation>`. WAL/SST/vlog/MANIFEST/CHECKPOINT/CHANGELOG/history MANIFEST/segment/bloom already call `crc_match_ok`. `verify_latest_pointer` still compared `crc32c(&mb) != expect_crc` inline. AS-IS treats any checksum as matching (flipped pointer reported clean). This slice names the gate on the **live remote verify path**: mismatch is a `LATEST` failure containing `crc mismatch`. The lie is **only the CRC hex line** (name intact, target MANIFEST bytes intact). XOR of an ASCII hex digit is not this tooth (`parse_latest_pointer` would fail as `bad pointer`).

The reader (`latest_manifest`) stays **walk-back** on mismatch: a lying pointer must not serve the named generation. That is not an Err tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- `RemoteTier::put_manifest` writes `LATEST` after the immutable generation.
- `RemoteTier::verify` calls `verify_latest_pointer` before walking segments.
- RFC-0086 P2.1 / RFC-0088 P1.2 named this remaining site. `remote_verify_flags_corrupt_latest` rewrites the hex to `ffffffff` (RFC-0060 P2.12) and does not pin `crc_match_ok`.

## Problems This Solves

- **Problem:** LATEST CRC lived in glue next to the pointer parser.
- **Problem:** AS-IS would report verify clean after a CRC-hex lie.
- **Problem:** history MANIFEST / segment / bloom used the named gate; the generation pointer did not.

## Proposed Solution

- Production `verify_latest_pointer` and `latest_manifest` call `crc_match_ok`. No new `*_kernel.rs`. Do not extract `db.rs`. Reader still walks back on mismatch.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live remote verify)
- [x] **P0.1** `verify_latest_pointer` + `latest_manifest` call `crc_match_ok` — status: `done`
- [x] **P0.2** Regression: live `put_manifest`, XOR CRC hex u32 only, verify names crc mismatch — status: `done` (`crc_mismatch_on_live_history_latest_is_not_ok`)

### P1 — next wave
- [x] **P1.1** Read-path `sidecar_may_affect` uses `crc_match_ok` (still fail-open: walk on mismatch) — status: `done` (RFC-0088 P1.1)
- [x] **P1.2** Walk-back still refuses a named older generation — status: `done` (`crc_mismatch_on_live_history_latest_walkback_refuses_named_older`)

### P2 — later
- [x] **P2.1** Collision axiom remains R-crc — status: `done` (`history_latest_crc_collision_axiom_remains`)
- [x] **P2.2** Content-addressed put read-back stays RFC-0092 (not this trailer gate) — status: `done` (`history_latest_put_readback_stays_rfc0092`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | LATEST CRC gated | done | history.rs verify_latest_pointer + latest_manifest | 2026-08-27 |
| P0.2 | p0 | CRC-hex lie is crc mismatch | done | crc_mismatch_on_live_history_latest_is_not_ok | 2026-08-27 |
| P1.1 | p1 | fail-open prune uses crc_match_ok | done | RFC-0088 P1.1 | 2026-08-28 |
| P1.2 | p1 | walk-back refuses named older | done | crc_mismatch_on_live_history_latest_walkback_refuses_named_older | 2026-08-28 |
| P2.1 | p2 | R-crc collision | done | history_latest_crc_collision_axiom_remains | 2026-08-28 |
| P2.2 | p2 | put read-back stays RFC-0092 | done | history_latest_put_readback_stays_rfc0092 | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_history_latest_is_not_ok`: production `RemoteTier::put_manifest` (real `LATEST` on Darwin). XOR only the stored CRC u32, rewrite as 8 hex digits (name line intact, MANIFEST bytes intact). `RemoteTier::verify` is not clean; a `LATEST` failure contains `crc mismatch`. `latest_manifest` still returns the generation (walk-back). Dropping `crc_match_ok` (AS-IS) would report verify clean. Does not submit io_uring SQEs.
  - Existing `remote_verify_flags_corrupt_latest` (`ffffffff` rewrite) is **not** this tooth.
  - P1.2 `crc_mismatch_on_live_history_latest_walkback_refuses_named_older`: two generations; `LATEST` names the older with XOR'd CRC hex. `latest_manifest` serves m2, never m1. AS-IS would serve the named older. `ffffffff` rewrite (`remote_manifest_generations_latest_and_walkback`) is RFC-0060 P2.12, not this tooth.
  - P2.1 `history_latest_crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `R-crc` stays in `never_floor`.
  - P2.2 `history_latest_put_readback_stays_rfc0092`: put resume identity stays RFC-0092 (`crc_match_ok` on two computed CRCs). This RFC does not take put identity. Byte-equal is RFC-0092 P2.1.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0089; RFC-0086 P2.1, RFC-0087 P1.2, RFC-0088 P1.2 marked done.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- ECC / CPU proof. CRC collision (R-crc). Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Making `latest_manifest` fail-closed (torn LATEST must walk back).
- Making the bloom prune fail-closed.
- Rocks coluna A/B. crates.io.
