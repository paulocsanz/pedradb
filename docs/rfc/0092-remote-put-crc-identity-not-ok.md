# RFC: 0092 — Remote history put CRC identity is not AlreadyPresent (never treat a flipped object as a resume no-op)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0091](0091-ops-warch-crc-mismatch-not-ok.md), [0046](0046-mvcc-history-tiering-s3.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk** — Sim2’s disk is a model. Pedra’s remote `put_segment` is content-addressed and idempotent: a name hit with **identical** bytes is `AlreadyPresent`. The identity check was `len` + `crc32c(have) == crc32c(bytes)` inline. AS-IS treats any checksum as matching (same-length flipped object accepted as a resume no-op). This slice names the gate: mismatch is `CorruptHistory` containing `crc mismatch`. The lie is **same-length payload XOR** (name/len intact). A different-length plant (`remote_segment_name_collision_fails_closed`) is not this tooth.

Byte-equal of the two buffers is P2 (collision axiom stays R-crc).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- `RemoteTier::put_segment` walks local bytes, then create-or-resume at the content-addressed name.
- Resume compared two computed CRC32Cs, not a stored trailer. RFC-0089 P2.2 named this leftover.
- Trailer gates (WAL/SST/history/warch) already call `crc_match_ok`.

## Problems This Solves

- **Problem:** resume identity lived in glue next to the put.
- **Problem:** AS-IS would report `AlreadyPresent` after a same-length payload lie.
- **Problem:** a length-mismatch plant is not the CRC tooth.

## Proposed Solution

- Production `put_segment` calls `crc_match_ok` on the two computed CRCs after the length check. No new `*_kernel.rs`. Do not extract `db.rs`. Do not add byte-equal in P0 (that would hide the AS-IS CRC tooth).

## Delivery slices (mandatory)

### P0 — must ship first (gate on live put resume)
- [x] **P0.1** `put_segment` calls `crc_match_ok` — status: `done`
- [x] **P0.2** Regression: live upload, XOR remote payload (len intact), re-put is crc mismatch — status: `done` (`crc_mismatch_on_live_history_put_is_not_ok`)

### P1 — next wave
- [x] **P1.1** `put_sidecar` uses `crc_match_ok` — status: `done` (RFC-0093)
- [x] **P1.2** Collision error still names the object — status: `done` (`crc_mismatch_on_live_history_put_names_the_object`)

### P2 — later
- [x] **P2.1** Resume also requires byte-equal (R-crc still never) — status: `done` (`history_put_resume_requires_byte_equal_after_crc`)
- [x] **P2.2** Collision axiom remains R-crc — status: `done` (`history_put_crc_collision_axiom_remains`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | put_segment gated | done | history.rs put_segment | 2026-08-27 |
| P0.2 | p0 | same-len payload lie is crc mismatch | done | crc_mismatch_on_live_history_put_is_not_ok | 2026-08-27 |
| P1.1 | p1 | put_sidecar CRC | done | RFC-0093 | 2026-08-27 |
| P1.2 | p1 | error names the object | done | crc_mismatch_on_live_history_put_names_the_object | 2026-08-28 |
| P2.1 | p2 | byte-equal resume | done | history_put_resume_requires_byte_equal_after_crc | 2026-08-28 |
| P2.2 | p2 | R-crc collision | done | history_put_crc_collision_axiom_remains | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_history_put_is_not_ok`: production `put_segment` (real remote object on Darwin). XOR one payload byte (length intact). Second `put_segment` is Err containing `crc mismatch`. Dropping `crc_match_ok` (AS-IS) would return `AlreadyPresent`. Does not submit io_uring SQEs.
  - Existing `remote_segment_name_collision_fails_closed` (different length) is **not** this tooth.
  - P1.2 `crc_mismatch_on_live_history_put_names_the_object`: same same-length lie; error contains `crc mismatch` and the content-addressed `seg-*.hist` name.
  - P2.1 `history_put_resume_requires_byte_equal_after_crc`: byte-equal follows `crc_match_ok` (never before). Identical re-put is `AlreadyPresent`. Same-length XOR still fails CRC first (P0 tooth). Sidecar byte-equal is RFC-0093 P1.2, not this tooth.
  - P2.2 `history_put_crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `R-crc` stays in `never_floor`. Byte-equal is not a collision theorem.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0092.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- ECC / CPU proof. CRC collision (R-crc). Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Byte-equal resume in this slice (P2.1).
- Rocks coluna A/B. crates.io.
