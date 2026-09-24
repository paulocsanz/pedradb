# RFC: 0080 — Verified profile does not admit a proven io_uring ring

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0058](0058-verified-mode-kernel-derived-fallback.md), [0062](0062-launch-readiness-remaining-gaps.md), [0074](0074-cqe-negative-res-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-uring` (not `never_floor`). Axis vs FDB Sim2: **G5 ring** — Sim2 has no io_uring. Pedra’s verified profile already lists `io_uring_ring` as Off (RFC-0058 P2.2: no proven ring model; constructors pin `StdEnv` / `IoUringEnv::posix()`). Production G1 is POSIX `fdatasync` (RFC-0062 / 0073). This slice names the gate on the **live verified open path**: `ring_model_admitted` is always false; a verified `ConcurrentDb` put cannot claim the ring is proven. AS-IS would admit a live ring inside verified.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. It does **not** restore production WAL onto the ring. `cqe_res_ok` stays on the test-Linux inject path (RFC-0074). Twin of the ring remains blocked. Full mode `PosixFallback` stays TCB.

## Background

- RFC-0058 P2.2: `profile_report` row `io_uring_ring` is Off. `ConcurrentDb::open_verified` uses `StdEnv`. CLI `PEDRA_VERIFIED=1` uses `IoUringEnv::posix()`.
- RFC-0074 named `cqe_res_ok` on the test-Linux ring. Production write/sync stay `pwrite` / `fdatasync_file`.
- A green verified open can still be rounded to “the ring is in the TCB and proven.”

## Problems This Solves

- **Problem:** ring-off lived as a static table row, not a decision the live verified engine calls.
- **Problem:** AS-IS would admit `verified_admits_ring(true)` (live ring inside verified).
- **Problem:** putting WAL back on the ring to “exercise CQE” would undo RFC-0062.

## Proposed Solution

- Pure `ring_model_admitted()` always false. AS-IS true.
- Pure `verified_admits_ring(want_ring)` = `want_ring && ring_model_admitted()`. AS-IS = `want_ring`.
- Lives in `verified.rs` (no new `*_kernel.rs`). `ConcurrentDb::open_verified` + `claim_uring_ring_proven` on the live engine. Profile Off must match the kernel. Do not restore WAL onto the ring.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live verified open)
- [x] **P0.1** `ring_model_admitted` + `verified_admits_ring` + AS-IS — status: `done`
- [x] **P0.2** `ConcurrentDb::claim_uring_ring_proven`; profile Off tracks the kernel — status: `done`
- [x] **P0.3** Regression: `open_verified` + put, claim is false; AS-IS would admit — status: `done` (`claim_uring_ring_refused_on_verified_open`)

### P1 — next wave
- [x] **P1.1** CLI `open_verified_db` names `verified_admits_ring` next to `posix()` — status: `done` (`open_verified_db`; `verified_flag_pins_the_profile_and_survives_reopen`)
- [x] **P1.2** Full mode still uses `PosixFallback` where the ring is unavailable — status: `done` (`full_uses_posix_fallback`; `full_mode_posix_fallback_when_ring_unavailable`)

### P2 — later
- [x] **P2.1** Ring twin stays blocked (R-uring) — status: `done` (`ring_twin_admitted`; no `verus/ring_model.rs`)
- [x] **P2.2** Do not put production WAL on SQE submit — status: `done` (`wal_on_sqe_admitted`; `IoUringFile` write/sync_data; `production_wal_is_not_on_sqe`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | ring_model_admitted + verified_admits_ring | done | verified.rs | 2026-08-27 |
| P0.2 | p0 | claim_uring_ring_proven + profile Off | done | concurrent.rs | 2026-08-27 |
| P0.3 | p0 | verified open tooth | done | claim_uring_ring_refused_on_verified_open | 2026-08-27 |
| P1.1 | p1 | CLI names the kernel | done | open_verified_db + verified_flag banner | 2026-08-28 |
| P1.2 | p1 | full PosixFallback | done | full_uses_posix_fallback + production_env | 2026-08-28 |
| P2.1 | p2 | twin blocked | done | ring_twin_admitted (no ring_model.rs twin) | 2026-08-28 |
| P2.2 | p2 | WAL stays off the ring | done | wal_on_sqe_admitted + production_wal_is_not_on_sqe | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `ring_model_admitted()` false; AS-IS true. `verified_admits_ring(true)` false; AS-IS true.
  - `claim_uring_ring_refused_on_verified_open`: `ConcurrentDb::open_verified`, `put`, `get` sees the value, `is_verified`, `claim_uring_ring_proven` is false. `profile_report` `io_uring_ring` is Off iff `!ring_model_admitted()`. Runs on Darwin. Does not submit SQEs.
  - P1.1 `verified_flag_pins_the_profile_and_survives_reopen`: `PEDRA_VERIFIED=1` banner names `posix()` and `verified_admits_ring=0`; `open_verified_db` calls `verified_admits_ring` then `IoUringEnv::posix()`.
  - P1.2 `full_mode_posix_fallback_when_ring_unavailable`: `full_uses_posix_fallback(false)` true; AS-IS false; `production_env()` backend is PosixFallback iff the kernel says fallback.
  - P2.1 `ring_model_is_not_admitted`: `ring_twin_admitted()` false; AS-IS true; `crates/pedradb-core/verus/ring_model.rs` absent.
  - P2.2 `production_wal_is_not_on_sqe`: live `IoUringEnv::posix()` put; `wal_on_sqe_admitted()` false; production `Write`/`sync_data` call the gate then POSIX `pwrite`/`fdatasync`. AS-IS would put WAL on SQE.
- **Telemetry / Analytics:** none — honesty invariant.
- **Documentation:** this RFC; `residuals.json` `R-uring` close-text + owner 0080.
- **Screenshots:** backend-only.

## Out of scope

- Restoring production WAL/SST onto io_uring (RFC-0062). Relitigating RFC-0074 CQE on dead wrappers.
- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Proving the ring. Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Rocks coluna A/B. crates.io.
