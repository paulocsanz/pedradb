# RFC: 0074 — io_uring CQE negative `res` is not Ok

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0061](0061-residuals-sel4-ironfleet.md), [0062](0062-launch-readiness-remaining-gaps.md), [0050](0050-world-in-tree-fdb-determinism.md), [0073](0073-fdatasync-rc-fail-closed.md)

**Residual:** `R-unsafe-uring` (not `never_floor`). Axis vs FDB Sim2: **G5 ring** — Sim2 has no io_uring. Pedra’s Linux crate still contains `unsafe submit_sqe`. **Production G1 is POSIX `fdatasync`** (RFC-0062 / 0073: `submit_and_wait` on every Ok was the coluna B tax). The only *live* SQE submit is the Linux **test** Env path used for CQE inject (`Write` / `sync_data` under `cfg(all(test, linux))`). This slice names the harvest gate on that path: negative CQE `res` is not Ok. AS-IS treats it as success (false Ok on fsync).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. It does **not** claim production WAL uses the ring. `SAFETY.md` is not a ∀ proof. The ring crate stays TCB.

## Background

- RFC-0062 P1.1: production `IoUringFile::write` = `pwrite(2)`; `sync_data` = `fdatasync_file` (0073). `uring_write` / `uring_fsync` were `dead_code` outside tests.
- RFC-0050 P0.2: `linux_cqe_eio_is_not_ok` injects `-EIO` into the harvest. Tests under `cfg(test)` still route `sync_data` through the ring so inject sees fsync.
- The predicate `res < 0` was inline in `UringState::{pwrite,fsync}`. A happy-path Env smoke does not hit it.

## Problems This Solves

- **Problem:** negative CQE could be rounded to Ok if the inline `if res < 0` drifts.
- **Problem:** a smoke `write_all`+`sync_data` on POSIX/fallback never calls the gate.
- **Problem:** `submit_complete_act`’s `unused_variables` allow was stolen by a misplaced `cqe_res_ok`.

## Proposed Solution

- Pure `cqe_res_ok(res)` = `res >= 0`. AS-IS always true.
- `UringState::{pwrite,fsync}` call it (existing `cqe_kernel`, no new TCB file).
- Linux **tests**: `Write::write` and `sync_data` use the ring so inject is live. Production stays POSIX.

## Delivery slices (mandatory)

### P0 — must ship first (gate on the live *test-Linux ring* path)
- [x] **P0.1** `cqe_res_ok` + AS-IS in `cqe_kernel`; `submit_complete_act` keeps its allow — status: `done`
- [x] **P0.2** `UringState::{pwrite,fsync}` call the gate; test-Linux `Write`/`sync_data` route to them — status: `done`
- [x] **P0.3** Regression: inject `-EIO`/`-ENOSPC` on the live ring; missing gate fails — status: `done` (`linux_cqe_eio_is_not_ok`; kernel `cqe_negative_res_is_not_ok` on every host)

### P1 — next wave
- [x] **P1.1** `linux_cqe_eio_is_not_ok` asserts `cqe_res_ok(-EIO)` — status: `done`
- [x] **P1.2** Production G1 POSIX documented (this RFC Background) — status: `done`

### P2 — later
- [x] **P2.1** Verus twin of `cqe_res_ok` (was allowlist) — status: `done` (paid single-artifact since 2026-09-09: Charon+Aeneas extract of the rustc body, `scripts/aeneas_cqe.sh`; the `verus/cqe_res.rs` twin and ghost block are deleted)
- [x] **P2.2** Ring model (R-uring) still blocked — status: `done` (`cqe_ring_model_admitted`; no `verus/ring_model.rs`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | cqe_res_ok + allow restored | done | cqe_kernel.rs | 2026-08-27 |
| P0.2 | p0 | pwrite/fsync + test-Linux Env | done | ring.rs + Write/sync_data | 2026-08-27 |
| P0.3 | p0 | inject -EIO on live ring | done | linux_cqe_eio_is_not_ok | 2026-08-27 |
| P1.1 | p1 | inject test asserts kernel | done | linux_cqe_eio_is_not_ok | 2026-08-27 |
| P1.2 | p1 | production G1 is POSIX | done | this RFC | 2026-08-27 |
| P2.1 | p2 | Verus twin | done | cqe_res.rs + catalog cqe_res | 2026-08-28 |
| P2.2 | p2 | ring model blocked | done | cqe_ring_model_admitted (no ring_model.rs) | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `cqe_negative_res_is_not_ok` (cqe_kernel): `cqe_res_ok(0)` true; negative false; AS-IS true on negative. Runs on Darwin.
  - `linux_cqe_eio_is_not_ok`: live `IoBackend::IoUring`, `inject_next_cqe_res(-EIO)` then `sync_data` is Err(EIO); `-ENOSPC` then `write_all` is Err. **Skipped off Linux.** Dropping `cqe_res_ok` from `fsync`/`pwrite` (returning Ok on negative res) fails this test.
  - P2.1 catalog pair `cqe_res` entry `cqe_res_ok` with Verus twin (freeze of twin files; `verus` not on PATH). `cqe_kernel.rs` leaves the TCB allowlist (now a catalog pair). No new `*_kernel.rs`.
  - P2.2 `cqe_ring_model_is_not_admitted`: `cqe_ring_model_admitted()` false; AS-IS true; the kernel `src/cqe_kernel.rs` is the single artifact (`verus/cqe_res.rs` deleted 2026-09-09); `verus/ring_model.rs` absent. Production WAL stays POSIX (RFC-0062 / 0080).
- **Telemetry / Analytics:** none — durability invariant.
- **Documentation:** this RFC; `residuals.json` `R-unsafe-uring` close-text + owner 0074 (SAFETY.md ≠ forall).
- **Screenshots:** backend-only.

## Out of scope

- Restoring production WAL/SST I/O onto the ring (RFC-0062).
- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Proving the ring / OS (R-uring, R-fsync-lie). Extracting `db.rs`.
- `never_floor`. New `*_kernel.rs`. Rocks coluna A/B. crates.io.
