# RFC: 0082 — MANIFEST CRC mismatch is not Ok (never load flipped inventory)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0060](0060-field-and-hardware-residuals.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk** — Sim2’s disk is a model. Pedra’s `MANIFEST-*` carries a trailing CRC32C; `CURRENT` may carry a CRC32C of that file. WAL (0076), SST (0077), vlog (0081) already call `crc_match_ok`. `manifest::load` / `decode` still compared `stored != computed` inline. AS-IS treats any checksum as matching (flipped SST inventory served on open). This slice names the gate on the **live `Db::open` recover path**: mismatch is `CorruptManifest`, never a table list.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- `Db::open` calls `manifest::load` (CURRENT pointer + MANIFEST decode) before SST recover.
- `verify.rs` already uses `crc_match_ok` on CURRENT; production `load` did not.
- FDB Sim2 does not CRC-walk a real MANIFEST. Pedra must not round a mismatch to an empty/wrong SST set.

## Problems This Solves

- **Problem:** MANIFEST/CURRENT CRC lived in glue; a drift could swallow bitrot as a clean inventory.
- **Problem:** AS-IS always matches (silent-wrong SST list).
- **Problem:** WAL/SST/vlog used the named gate; the recover inventory did not.

## Proposed Solution

- Production `load` (CURRENT file CRC) and `decode` (MANIFEST trailer) call `crc_match_ok`. No new `*_kernel.rs`. Do not extract `db.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live Db::open)
- [x] **P0.1** `load` + `decode` call `crc_match_ok` — status: `done`
- [x] **P0.2** Regression: live flush, CURRENT crc line lie (MANIFEST intact), `Db::open` is Err(crc); AS-IS would serve the key — status: `done` (`crc_mismatch_on_live_manifest_is_not_ok`)

### P1 — next wave
- [x] **P1.1** `Db::open` of a flipped WAL (0076 P1.2) — status: `done` (RFC-0083)
- [x] **P1.2** History MANIFEST uses the same gate — status: `done` (RFC-0086 P0 `crc_mismatch_on_live_history_manifest_is_not_ok`)

### P2 — later
- [x] **P2.1** Catalog / Verus token — status: `done` (catalog `crc_match` callers `manifest.rs` + `history.rs`)
- [x] **P2.2** Collision axiom remains R-crc — status: `done` (`manifest_crc_collision_axiom_remains`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | load + decode gated | done | manifest.rs | 2026-08-27 |
| P0.2 | p0 | CURRENT crc lie, MANIFEST intact | done | crc_mismatch_on_live_manifest_is_not_ok | 2026-08-27 |
| P1.1 | p1 | Db::open flipped WAL | done | RFC-0083 | 2026-08-27 |
| P1.2 | p1 | history MANIFEST | done | RFC-0086 crc_mismatch_on_live_history_manifest_is_not_ok | 2026-08-28 |
| P2.1 | p2 | catalog + Verus | done | crc_match callers += manifest.rs, history.rs | 2026-08-28 |
| P2.2 | p2 | R-crc collision | done | manifest_crc_collision_axiom_remains | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_manifest_is_not_ok`: `Db::open` + `put` + `flush` (production CURRENT+MANIFEST). Rewrite only the CURRENT CRC line to `ffffffff` (MANIFEST bytes intact, still parseable). `Db::open` is Err containing `crc mismatch`. Dropping `crc_match_ok` on `load` (AS-IS) would open and serve `k`. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.2 `crc_mismatch_on_live_history_manifest_is_not_ok` (RFC-0086 P0): `history/MANIFEST` trailer XOR; `verify_history_manifest` / `HistoryTier::open` is crc mismatch. Same `crc_match_ok` as LSM MANIFEST. No new `*_kernel.rs`.
  - P2.1 catalog pair `crc_match` lists `manifest.rs` and `history.rs` as callers of `crc_match_ok` (same Verus twin as RFC-0076; freeze of twin files; `verus` not on PATH).
  - P2.2 `manifest_crc_collision_axiom_remains`: `crc_collision_admitted()` false; AS-IS true; `residuals.json` still lists `R-crc` in `never_floor`.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text + owner 0082.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- ECC / CPU proof. CRC collision (R-crc). Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Rocks coluna A/B. crates.io.
