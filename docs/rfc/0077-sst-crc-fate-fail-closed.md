# RFC: 0077 — SST file CRC fate is not glue (mismatch never a table)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0061](0061-residuals-sel4-ironfleet.md), [0076](0076-crc-mismatch-not-ok.md), [0056](0056-one-hundred-percent-delivery.md)

**Residual:** `R-glue` (not `never_floor`). Axis vs FDB Sim2: **G3 disk** — Sim2’s disk is a model. Pedra’s SST files carry a trailing CRC32C and fail-stop on bitrot. That fate (`match → strip trailer` / `tiny legacy → whole buffer` / `else reject`) lived inline in `SstTable::decode`. AS-IS always strips the trailer (corruption served as a table). This slice names the gate on the **live SST open path**: a flipped modern file is Reject, never a `SstTable`.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Zero-glue is a trajectory, not a theorem. This does not extract `db.rs`. CRC32C collision is still R-crc. Bits after the last scrub stay R-hardware. This is not an ECC proof.

## Background

- RFC-0076 named `crc_match_ok` on the WAL reader. SST `decode` still compared `stored == computed` in the handler, with a tiny-file legacy fallback (`buf.len() < 32`).
- Coverage map: SST `*.sst` CRC is verified-on-read (`SstTable::open_on`) and on-scrub. The predicate was glue.
- FDB Sim2 does not CRC-walk a real SST forest. Pedra must not round a mismatch to a parsed table.

## Problems This Solves

- **Problem:** SST CRC fate was inline in `table.rs`; a drift could swallow bitrot as legacy-without-CRC.
- **Problem:** AS-IS always `StripTrailer` (silent-wrong table).
- **Problem:** R-glue still has no extracted live handler decision on the SST open path.

## Proposed Solution

- Pure `sst_crc_fate(stored, computed, buf_len)` in the existing SST kernel (`scan_kernel.rs` — no new `*_kernel.rs`).
- Production `SstTable::decode` matches on the fate. No `db.rs` extract.

## Delivery slices (mandatory)

### P0 — must ship first (gate on the live SST open)
- [x] **P0.1** `sst_crc_fate` + AS-IS in `scan_kernel.rs` — status: `done`
- [x] **P0.2** `SstTable::decode` calls the fate — status: `done`
- [x] **P0.3** Regression: live write, flip payload, open is CRC mismatch; AS-IS would strip — status: `done` (`crc_mismatch_on_live_sst_is_not_ok`)

### P1 — next wave
- [x] **P1.1** SST data-block CRC (per-block) uses the same admit — status: `done` (`sst_block_crc_ok`; v5 writer; `crc_mismatch_on_live_sst_block_is_not_ok`)
- [x] **P1.2** `Db::open` of a flipped live SST is Err / does not serve the flipped key — status: `done` (`crc_mismatch_on_live_sst_db_open_is_not_ok`)

### P2 — later
- [x] **P2.1** Catalog pair / Verus token for `sst_crc_fate` (scan_guard stays F167) — status: `done` (twin `verus/sst_crc_fate.rs` deletado 2026-09-09, mirror sweep: single-artifact pago pelo extrato Aeneas do corpo rustc `scripts/aeneas_scan.sh`, `ScanKernel.lean` sem sorry + catalog `sst_crc`)
- [x] **P2.2** Zero-glue remains a trajectory — status: `done` (`zero_glue_admitted`; `zero_glue_is_a_trajectory`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | sst_crc_fate + AS-IS | done | scan_kernel.rs | 2026-08-27 |
| P0.2 | p0 | decode gated | done | table.rs | 2026-08-27 |
| P0.3 | p0 | flipped SST is mismatch | done | crc_mismatch_on_live_sst_is_not_ok | 2026-08-27 |
| P1.1 | p1 | per-block CRC | done | sst_block_crc_ok + crc_mismatch_on_live_sst_block_is_not_ok | 2026-08-28 |
| P1.2 | p1 | Db::open flipped SST | done | crc_mismatch_on_live_sst_db_open_is_not_ok | 2026-08-28 |
| P2.1 | p2 | catalog + Verus | done | sst_crc_fate.rs + catalog sst_crc | 2026-08-28 |
| P2.2 | p2 | zero-glue trajectory | done | zero_glue_admitted | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `sst_crc_fate(1, 2, 100)` is `Reject`; AS-IS is `StripTrailer`. Tiny mismatch is `WholeBuffer`.
  - `crc_mismatch_on_live_sst_is_not_ok`: production `write_sst` (real file), flip a payload byte, `SstTable::open` is Err naming CRC mismatch. Runs on Darwin.
  - P1.1 `sst_block_crc_ok(1,2)` false; AS-IS true. `crc_mismatch_on_live_sst_block_is_not_ok`: `write_sst` (v5), flip a data-block byte, rewrite the *file* trailer so `sst_crc_fate` is StripTrailer; `SstTable::open` is still CRC mismatch. v4 files stay readable (no per-block CRC). L0 uncompressed remains v3. No new `*_kernel.rs`.
  - P1.2 `crc_mismatch_on_live_sst_db_open_is_not_ok`: `Db::open` + `put` + `flush`; flip an on-disk `*.sst` payload byte; `Db::open` is Err containing `CRC mismatch` (does not serve the key). AS-IS would strip the trailer.
  - P2.1 catalog pair `sst_crc` entry `sst_crc_fate` with Verus twin (freeze of twin files; `verus` not on PATH). `scan_guard` stays F167 (`scan_reads_file`); its twin `verus/scan_guard.rs` was deleted 2026-09-09 — the 4 F167 pairs are single-artifact paid by the Aeneas extract of the rustc bodies (`scripts/aeneas_scan.sh`, `ScanKernel.lean` sorry-free). No new `*_kernel.rs`.
  - P2.2 `zero_glue_is_a_trajectory`: `zero_glue_admitted()` false; AS-IS true; `src/db.rs` present; `glue.db_rs_extracted` false; `R-glue` remains.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-glue` close-text + owner 0077; `R-hardware` close-text names the SST tooth.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- ECC / CPU proof. CRC collision (R-crc). Proving the drive (R-fsync-lie).
- Rocks coluna A/B. crates.io. Restoring production WAL onto io_uring.
