# RFC: 0073 — `fdatasync` nonzero rc is not Ok (posix island)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0061](0061-residuals-sel4-ironfleet.md), [0036](0036-tikv-rocks-2x-fdatasync.md), [0052](0052-dst-inside-boxes.md)

**Residual:** `R-unsafe-posix` (not `never_floor`). Axis vs FDB Sim2: **G1 / G3 syscall** — Sim2 never issues libc `fdatasync`; Pedra’s WAL G1 does, in an `unsafe` FFI island (`pedradb-posix`). The island already mapped `rc == 0` → Ok, else `last_os_error()`. That predicate was inline next to the `unsafe` call. AS-IS treats any rc as success (silent skip of the barrier). This slice names the gate: nonzero rc is not Ok.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. `SAFETY.md` is not a ∀ proof. Miri does not execute the real syscall. The OS can still lie on `rc == 0` (R-fsync-lie).

## Background

- `pedradb-core` is `forbid(unsafe_code)`. WAL G1 calls `pedradb_posix::fdatasync_file` (libSystem / Linux `fdatasync`, not Darwin `F_FULLFSYNC`).
- RFC-0061: the island has SAFETY.md + `miri-unsafe-islands.sh`. Residual class `continuous`.
- FDB Sim2 `AsyncFileNonDurable` is a model. Pedra’s production barrier is this FFI.

## Problems This Solves

- **Problem:** `rc == 0` lived next to `unsafe { fdatasync(...) }` with no named decision the tests can tooth.
- **Problem:** AS-IS would return Ok on EIO/EINTR and skip the barrier.
- **Problem:** `R-unsafe-posix` close-text did not name a production fail-closed besides SAFETY.md.

## Proposed Solution

- Pure `fdatasync_rc_ok(rc)` = `rc == 0`. AS-IS always true.
- Production `fdatasync_file` (unix) calls it. No new `*_kernel.rs` (TCB freeze).

## Delivery slices (mandatory)

### P0 — must ship first (gate on the live posix path)
- [x] **P0.1** `fdatasync_rc_ok` + AS-IS — status: `done`
- [x] **P0.2** unix `fdatasync_file` uses the gate — status: `done`
- [x] **P0.3** Regression: real `File` `fdatasync_file` Ok; nonzero rc is not Ok; AS-IS would — status: `done` (`fdatasync_nonzero_rc_is_not_ok`)

### P1 — next wave
- [x] **P1.1** `fsync_file` / `sync_dir_fd` share the same rc gate where they FFI — status: `done` (`posix_rc_to_io`; `fsync_and_dirfd_share_rc_gate`)
- [x] **P1.2** Miri test of `fdatasync_rc_ok` (safe fn) in `miri-unsafe-islands.sh` — status: `done` (`fdatasync_rc_ok_is_safe_predicate`)

### P2 — later
- [x] **P2.1** Catalog pair + Verus twin — status: `done` (`verus/fdatasync_rc.rs` + catalog `fdatasync_rc`; mirror + runner deletados 2026-09-09 — fire 780: single-artifact via `aeneas_posix.sh` extract de `lib.rs` (`fdatasync_rc_ok`/`_as_is` no `PosixKernel.lean`, 0 sorry; SOURCE = git HEAD — lib.rs é dirty da sessão de performance))
- [x] **P2.2** EINTR retry policy named (RFC-0015 H1) — status: `done` (`fdatasync_eintr_retry_admitted`; `fdatasync_eintr_is_not_retried_as_ok`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | fdatasync_rc_ok + AS-IS | done | pedradb-posix/src/lib.rs | 2026-08-27 |
| P0.2 | p0 | fdatasync_file uses gate | done | fdatasync_file unix | 2026-08-27 |
| P0.3 | p0 | nonzero rc tooth | done | fdatasync_nonzero_rc_is_not_ok | 2026-08-27 |
| P1.1 | p1 | other posix FFI gates | done | fsync_and_dirfd_share_rc_gate | 2026-08-28 |
| P1.2 | p1 | Miri on the safe fn | done | fdatasync_rc_ok_is_safe_predicate + miri-unsafe-islands.sh | 2026-08-28 |
| P2.1 | p2 | catalog + Verus | done | fdatasync_rc.rs + catalog fdatasync_rc | 2026-08-28 |
| P2.2 | p2 | EINTR policy | done | fdatasync_eintr_retry_admitted + fdatasync_eintr_is_not_retried_as_ok | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `fdatasync_rc_ok(0)` true; `fdatasync_rc_ok(-1)` false; AS-IS true on -1.
  - `fdatasync_nonzero_rc_is_not_ok`: create a real file, write, `fdatasync_file` Ok (production path); kernel teeth as above.
  - P1.1 `fsync_and_dirfd_share_rc_gate`: live `fsync_file` + `sync_dir_fd` Ok; share `fdatasync_rc_ok` (Linux FFI `fsync`; Darwin `fsync_file` stays `sync_all` / F_FULLFSYNC). AS-IS would ignore nonzero rc.
  - P1.2 `fdatasync_rc_ok_is_safe_predicate`: safe fn only. `miri-unsafe-islands.sh` runs that test by name.
  - P2.1 catalog pair `fdatasync_rc` entry `fdatasync_rc_ok` with Verus twin (freeze of twin files; `verus` not on PATH). No new `*_kernel.rs`.
  - P2.2 `fdatasync_eintr_is_not_retried_as_ok`: `fdatasync_eintr_retry_admitted()` false; AS-IS true; production `fdatasync_file` is one syscall then the rc gate. Err is uncertain (H1), not “record absent”.
- **Telemetry / Analytics:** none — durability invariant.
- **Documentation:** this RFC; `residuals.json` `R-unsafe-posix` close-text + owner 0073 (still names SAFETY.md ≠ forall).
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Proving the OS / drive (R-fsync-lie). Darwin `F_FULLFSYNC`. Extracting `db.rs`.
- `never_floor`. New `*_kernel.rs`. Rocks coluna A/B. crates.io.
