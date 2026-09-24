# RFC: 0155 — Silent-wrong fail-closed board (napply retry, glue, lock)

**Status:** done
**Updated:** 2026-08-30
**Parents:** [0072](0072-l28-real-tcp-durability-kernel.md), [0077](0077-sst-crc-fate-fail-closed.md), [0071](0071-group-publish-after-wal-durable.md), [0151](0151-three-teeth-as-is-verus-dst.md), [0152](0152-live-queued-vote-ae-catalog-kernel.md)

**Residual:** nine named silent-wrong rows stay published (`R-glue`, `R-group-glue`, `R-swarm-real`, `R-fsync-lie`, `R-unsafe-posix`, `R-unsafe-capi`, `R-es`, `R-crc`, `R-uring`). `R-crc` stays `never_floor`. This RFC does **not** extract `db.rs`, raise PCT default depth, invent a TCG guest, or delete never_floor ids.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, “perfeito”, “acabou”, or seL4-class. Honest close is a fail-closed kernel plus a named shipped-path plant **or** a published residual row. REAL TCP retry-success is not ∀ traces.

## Background

Nine silent-wrong weak points already have inhabitants. Eight of them already refuse the overclaim (fail-closed kernel + plant, or `never_floor`). Two of those kernels were **not cataloged** (`zero_glue_admitted`, `lock_interleavings_admitted`). The remaining closable hole is the L28 REAL TCP harness: `l28_real_tcp.rs` `run()` retries `cluster_real` 3× and treats a later `napply=1` as success, hiding an earlier `napply=0`. That retry is a campaign flake hedge, not a theorem that every TCP schedule recovers the removed replica.

Catalog freeze at start of this slice: pairs include `media_durable`, `fdatasync_rc`, `c_len`, `liveness_claim`, `crc_match`, `cqe_res`. Glue TCB stays (`db_rs_extracted=false`). PCT campaign default depth stays 2.

## Problems This Solves

- **Problem:** a 3-attempt TCP harness success can be rounded to “∀ TCP napply.”
- **Problem:** `zero_glue_admitted` / `lock_interleavings_admitted` refuse the overclaim in code but were missing catalog `live_callers` + three-teeth freeze.
- **Problem:** the nine residual rows named the original RFCs; the board that owns “silent-wrong is not closed” was not 0155.

## Proposed Solution

- New kernel `l28_tcp_napply_retry_admitted(attempts, napply_ok)` always false. AS-IS admits `attempts >= 1 && napply_ok`. Production `cluster_real` live-calls it on `--remove-member` (attempts=1 in-process). The harness still retries 3×; retry-success is not a kernel skip and still requires `l28_tcp_napply_ok` on the successful line.
- Catalog `zero_glue` and `lock_interleavings` (class-C freeze of kernels that already exist). `Db::claim_zero_glue` is the new live caller for R-glue. `ConcurrentDb::claim_lock_interleavings_proven` already exists — do not duplicate.
- Residual rows stay. Close-text names RFC-0155 as the board that names the fail-closed inhabitant. `never_floor` still lists R-cpu R-rustc R-verus R-crc R-deps R-extract.

## Delivery slices (mandatory)

### P0 — L28 napply retry is not ∀ TCP

- [x] **P0.1** `l28_tcp_napply_retry_admitted` always false; AS-IS `attempts >= 1 && napply_ok` — status: `done`
- [x] **P0.2** `cluster_real` live-calls the kernel on `--remove-member`; `l28_real_tcp::run` records attempt count and does not treat 3-attempt success as a kernel skip — status: `done`
- [x] **P0.3** Named plant `l28_real_tcp_removed_recover_apply` (extend; no second cluster) + unit tooth `l28_tcp_napply_retry_admitted_is_not_forall` — status: `done`

### P1 — remaining TCB named on live path

- [x] **P1.1** R-glue: `Db::claim_zero_glue` + catalog `zero_glue` + plant `zero_glue_admitted_on_live_db_is_not_ok` — status: `done`
- [x] **P1.2** R-group-glue: catalog `lock_interleavings` live_callers `claim_lock_interleavings_proven`; plant `claim_lock_interleavings_refused_after_put` — status: `done`
- [x] **P1.3** R-fsync-lie: residual close-text names 0155; existing `media_durable` plant remains — status: `done`
- [x] **P1.4** R-unsafe-posix: residual close-text names 0155; existing `fdatasync_rc` plant remains — status: `done`
- [x] **P1.5** R-unsafe-capi: residual close-text names 0155; existing `c_len` plant remains — status: `done`
- [x] **P1.6** R-es: residual close-text names 0155; existing `liveness_claim` plant remains — status: `done`
- [x] **P1.7** R-crc: never_floor row stays; close-text names 0155 — status: `done`
- [x] **P1.8** R-uring: residual close-text names 0155; existing ring refuse remains — status: `done`

### P2 — Verus twins + freeze

- [x] **P2.1** Verus twins `l28_tcp_napply_retry_admitted` / `lock_interleavings_admitted` / `zero_glue_admitted`; catalog `three_teeth: true` for the three new pairs — status: `done`
- [x] **P2.2** never_floor six ids still present; `db_rs_extracted` false; PCT default depth stays 2 — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | napply retry kernel always false | done | `l28_tcp_napply_retry_admitted` | 2026-08-30 |
| P0.2 | p0 | cluster_real + run_counted live path | done | `cluster_real.rs` / `l28_real_tcp.rs` | 2026-08-30 |
| P0.3 | p0 | shipped TCP plant + unit tooth | done | `l28_real_tcp_removed_recover_apply` / `l28_tcp_napply_retry_admitted_is_not_forall` | 2026-08-30 |
| P1.1 | p1 | catalog zero_glue + Db claim | done | `claim_zero_glue` / `zero_glue_admitted_on_live_db_is_not_ok` | 2026-08-30 |
| P1.2 | p1 | catalog lock_interleavings | done | existing `claim_lock_interleavings_proven` | 2026-08-30 |
| P1.3 | p1 | R-fsync-lie names 0155 | done | residuals `media_durable` | 2026-08-30 |
| P1.4 | p1 | R-unsafe-posix names 0155 | done | residuals `fdatasync_rc` | 2026-08-30 |
| P1.5 | p1 | R-unsafe-capi names 0155 | done | residuals `c_len` | 2026-08-30 |
| P1.6 | p1 | R-es names 0155 | done | residuals `liveness_claim` | 2026-08-30 |
| P1.7 | p1 | R-crc never_floor stays | done | residuals `crc_match` | 2026-08-30 |
| P1.8 | p1 | R-uring names 0155 | done | residuals `cqe_res` | 2026-08-30 |
| P2.1 | p2 | Verus twins + three_teeth | done | l28 paid by Aeneas extract (`aeneas_l28.sh`, mirror deleted); `group_commit.rs` / `sst_crc_fate.rs` | 2026-08-30 |
| P2.2 | p2 | never_floor + extract + PCT d=2 | done | residuals freeze | 2026-08-30 |

## Acceptance Criteria

- **Tests**
  - `l28_tcp_napply_retry_admitted(1, true)` is false; AS-IS true. `l28_tcp_napply_retry_admitted_is_not_forall`.
  - `l28_real_tcp_removed_recover_apply` uses `run_counted`, still asserts `l28_tcp_napply_ok`, asserts `!l28_tcp_napply_retry_admitted(attempts, napply_ok)` even when napply=1, and asserts the AS-IS dente.
  - `cluster_real --remove-member` calls `l28_tcp_napply_retry_admitted(1, napply_ok)` and exits 1 if it were ever true.
  - `zero_glue_admitted_on_live_db_is_not_ok`: open+put then `!db.claim_zero_glue()`; AS-IS true.
  - `claim_lock_interleavings_refused_after_put` remains the lock plant.
  - `python3 scripts/formal/pedra_formal.py --lint` ends `0 fail`. Catalog names `l28_napply_retry`, `zero_glue`, `lock_interleavings`.
- **Telemetry / Analytics:** none — fail-closed invariant. `cluster_real` still prints the fingerprint line.
- **Documentation:** this RFC; `residuals.json` close-text for the nine ids names 0155. Residual rows not deleted.
- **Screenshots:** backend-only.

## Out of scope

- Extract `db.rs` (`glue.db_rs_extracted` stays false). Raise PCT default depth above 2. Invent a TCG guest / SSH.
- Delete `never_floor` ids: R-cpu, R-rustc, R-verus, R-crc, R-deps, R-extract. Delete residual rows R-glue, R-group-glue, R-swarm-real, R-fsync-lie, R-unsafe-posix, R-unsafe-capi, R-es, R-crc, R-uring.
- rustfmt `lib.rs`. Benches / RFC-0149 / 0153 / 0154 CHV. Pedra vs Rocks `WriteOptions.sync=true`.
- “Garantia total”, “sem bugs”, seL4, “perfeito”, “acabou”. ∀ TCP traces. ∀ OS lock interleavings.
