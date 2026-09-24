# RFC: 0093 — Remote bloom sidecar put CRC identity is not a no-op (never treat a flipped filter as a resume)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0092](0092-remote-put-crc-identity-not-ok.md), [0088](0088-history-bloom-crc-mismatch-not-ok.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk** — Sim2’s disk is a model. Pedra’s `put_sidecar` resumes a remote `<name>.bloom` when `len` + `crc32c(have) == crc32c(bytes)`. `put_segment` already calls `crc_match_ok` (RFC-0092). AS-IS treats any checksum as matching (same-length flipped sidecar accepted as a resume no-op). This slice names the gate: mismatch is `CorruptHistory` containing `crc mismatch`. The lie is **same-length payload XOR** of the remote `.bloom` (segment object intact). RFC-0092’s `.hist` tooth is not this tooth.

Byte-equal remains P2 (collision axiom stays R-crc). Fail-open prune is RFC-0091 P2.1.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- `put_segment` ships the bloom sidecar after the segment object.
- Resume compared two computed CRC32Cs inline. RFC-0092 P1.1 named this leftover.

## Problems This Solves

- **Problem:** sidecar resume identity lived in glue.
- **Problem:** AS-IS would no-op after a same-length sidecar lie.
- **Problem:** the `.hist` gate does not cover `.bloom`.

## Proposed Solution

- Production `put_sidecar` calls `crc_match_ok` after the length check. No new `*_kernel.rs`. Do not extract `db.rs`. Do not add byte-equal in P0.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live sidecar resume)
- [x] **P0.1** `put_sidecar` calls `crc_match_ok` — status: `done`
- [x] **P0.2** Regression: live upload, XOR remote `.bloom` payload (len intact), re-put is crc mismatch — status: `done` (`crc_mismatch_on_live_history_sidecar_put_is_not_ok`)

### P1 — next wave
- [x] **P1.1** Collision error still names the `.bloom` object — status: `done` (`crc_mismatch_on_live_history_sidecar_put_names_the_object`)
- [x] **P1.2** Resume also requires byte-equal (R-crc still never) — status: `done` (`history_sidecar_put_resume_requires_byte_equal_after_crc`)

### P2 — later
- [x] **P2.1** Collision axiom remains R-crc — status: `done` (`history_sidecar_put_crc_collision_axiom_remains`)
- [x] **P2.2** Fail-open prune stays fail-open — status: `done` (`history_sidecar_put_prune_stays_fail_open`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | put_sidecar gated | done | history.rs put_sidecar | 2026-08-27 |
| P0.2 | p0 | same-len sidecar lie is crc mismatch | done | crc_mismatch_on_live_history_sidecar_put_is_not_ok | 2026-08-27 |
| P1.1 | p1 | error names the object | done | crc_mismatch_on_live_history_sidecar_put_names_the_object | 2026-08-28 |
| P1.2 | p1 | byte-equal resume | done | history_sidecar_put_resume_requires_byte_equal_after_crc | 2026-08-28 |
| P2.1 | p2 | R-crc collision | done | history_sidecar_put_crc_collision_axiom_remains | 2026-08-28 |
| P2.2 | p2 | prune stays fail-open | done | history_sidecar_put_prune_stays_fail_open | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_history_sidecar_put_is_not_ok`: production `put_segment` (real remote `.bloom` on Darwin). XOR one payload byte of the sidecar only (length intact, `.hist` intact). Second `put_segment` is Err containing `crc mismatch`. Dropping `crc_match_ok` (AS-IS) would no-op the sidecar. Does not submit io_uring SQEs.
  - `crc_mismatch_on_live_history_put_is_not_ok` (`.hist`) is **not** this tooth.
  - P1.1 `crc_mismatch_on_live_history_sidecar_put_names_the_object`: same same-length lie; error contains `crc mismatch` and the `.bloom` object name.
  - P1.2 `history_sidecar_put_resume_requires_byte_equal_after_crc`: byte-equal follows `crc_match_ok` in `put_sidecar` (never before). Identical re-put is `AlreadyPresent`. Same-length XOR still fails CRC first (P0 tooth). `.hist` byte-equal is RFC-0092 P2.1, not this tooth.
  - P2.1 `history_sidecar_put_crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `R-crc` stays in `never_floor`.
  - P2.2 `history_sidecar_put_prune_stays_fail_open`: prune stays RFC-0088 fail-open (`sidecar_may_affect` walks on trailer lie); scrub stays fail-closed. This RFC does not fail-close prune.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0093; RFC-0092 P1.1 marked done.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- ECC / CPU proof. CRC collision (R-crc). Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Making the history bloom prune fail-closed.
- Rocks coluna A/B. crates.io.
