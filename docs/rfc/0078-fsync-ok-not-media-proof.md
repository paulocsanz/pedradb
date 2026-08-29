# RFC: 0078 — `fsync` Ok is not a media proof (lying OS still drops)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0052](0052-dst-inside-boxes.md), [0073](0073-fdatasync-rc-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-fsync-lie` (not `never_floor`). Axis vs FDB Sim2: **G3 disk** — Sim2’s disk is `AsyncFileNonDurable` (a model). Pedra’s G1 is POSIX `fdatasync` (RFC-0073: nonzero rc is not Ok). The OS can still return 0 and drop the bytes. `RecordingEnv::SyncPolicy::Lying` is the in-process model of that undetectable lie. This slice names the two teeth: pending bytes promote only when the Env is honest; a live `Db` put after `fdatasync` Ok still **refuses** `claim_media_durable`.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. This does **not** detect a lying kernel. `RecordingEnv::Lying` stays the model (do not delete it). TCG/det_io remain the other box (R-tcg-guest). Recovery kernels cover the model, not the drive.

## Background

- RFC-0073: `fdatasync_rc_ok(rc)==(rc==0)` on the live posix island. Residual text already said the OS can lie on rc==0.
- RFC-0011 P2.2: `SyncPolicy::Lying` returns Ok from `sync_*` without promoting pending. Crash drops the write. World/sim already plant this.
- The promote vs no-promote decision was inline (`if policy == Honest`). A live `Db` had no API that *names* “fsync Ok ≠ media proof.”
- FDB Sim2 also does not prove the drive. Pedra must not round `fdatasync` 0 to ECC.

## Problems This Solves

- **Problem:** `sync_data` promote lived in glue; AS-IS would promote even when Lying (crash recovers a lie).
- **Problem:** production `Db` after a durable put had no fail-closed claim that this is not a drive theorem.
- **Problem:** R-fsync-lie close-text pointed only at TCG/det_io, not a kernel the live engine calls.

## Proposed Solution

- Pure `fsync_promotes_pending(os_honest)` = `os_honest`. AS-IS always true.
- Pure `media_durable_admitted(fsync_ok)` always false. AS-IS = `fsync_ok`.
- Lives in existing `group_commit_kernel` (no new `*_kernel.rs`). `RecordingEnv::sync_data` promotes only through the first gate. Production `Db::claim_media_durable` calls the second after a real put. Do not extract `db.rs`. Do not restore WAL onto io_uring.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live Db + Lying Env)
- [x] **P0.1** `fsync_promotes_pending` + `media_durable_admitted` + AS-IS — status: `done`
- [x] **P0.2** `RecordingEnv::sync_data` uses the promote gate; `Db::claim_media_durable` on the live engine — status: `done`
- [x] **P0.3** Regression: StdEnv put then claim is false; Lying put+crash get is None — status: `done` (`claim_media_durable_refused_after_fsync_ok`, `lying_fsync_does_not_promote_pending`)

### P1 — next wave
- [x] **P1.1** World `SyncPolicy::Lying` plant names the kernel — status: `done` (`world_lying_fsync_plant_names_kernel`)
- [x] **P1.2** det_io drop_fsync stays a different box (do not AND with Lying in one run — RFC-0052) — status: `done` (`stacked_fsync_liars_admitted`; `world_run_refuses_stacked_fsync_liars`; `det_io_status.sh`)

### P2 — later
- [x] **P2.1** Catalog / Verus token for the two fns — status: `done` (`verus/group_commit.rs` + catalog `fsync_promote` / `media_durable`)
- [x] **P2.2** TCG guest still R-tcg-guest; this RFC does not invent a guest — status: `done` (`fsync_lie_closes_tcg_guest`; `world_fsync_lie_does_not_invent_tcg_guest`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | promote + media_durable kernels | done | group_commit_kernel.rs | 2026-08-27 |
| P0.2 | p0 | RecordingEnv + Db claim wired | done | recording.rs + db.rs | 2026-08-27 |
| P0.3 | p0 | StdEnv claim + Lying crash teeth | done | named tests | 2026-08-27 |
| P1.1 | p1 | World names the kernel | done | world_lying_fsync_plant_names_kernel | 2026-08-28 |
| P1.2 | p1 | det_io XOR Lying | done | stacked_fsync_liars_admitted + world_run_refuses_stacked_fsync_liars + det_io_status.sh | 2026-08-28 |
| P2.1 | p2 | catalog + Verus | done | group_commit.rs fsync_promotes_pending + media_durable_admitted | 2026-08-28 |
| P2.2 | p2 | no invented guest | done | fsync_lie_closes_tcg_guest + world_fsync_lie_does_not_invent_tcg_guest | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `fsync_promotes_pending(false)` is false; AS-IS true. `media_durable_admitted(true)` is false; AS-IS true.
  - `claim_media_durable_refused_after_fsync_ok`: `Db::open` (StdEnv), `put`, `get` sees the value, `claim_media_durable()` is false. Runs on Darwin.
  - `lying_fsync_does_not_promote_pending`: `Db::open_with_env` + `RecordingEnv::Lying`, `put`, crash, reopen, `get` is None. AS-IS promote would recover the key.
  - P1.1 `world_lying_fsync_plant_names_kernel`: World-crate `StoreCluster` on `RecordingEnv::Lying`, put, crash, key gone; `fsync_promotes_pending(false)` false; AS-IS would promote. Refuses to run if det_io is preloaded.
  - P1.2 `world_run_refuses_stacked_fsync_liars`: live `World::run` `claim_stacked_fsync_liars` false; AS-IS `stacked_fsync_liars_admitted_as_is(true,true)` would admit. `det_io_status.sh` names `stacked_fsync_liars_admitted` and `box=det_io_preload xor RecordingEnv::Lying`.
  - P2.1 catalog pairs `fsync_promote` / `media_durable` with Verus twins in `verus/group_commit.rs` (freeze of twin files; `verus` not on PATH).
  - P2.2 `world_fsync_lie_does_not_invent_tcg_guest`: live `World::run` `claim_tcg_guest` false; `fsync_lie_closes_tcg_guest` false; AS-IS would claim 0078 closed TCG. No guest invented.
- **Telemetry / Analytics:** none — honesty invariant.
- **Documentation:** this RFC; `residuals.json` `R-fsync-lie` close-text + owner 0078. Lying policy remains.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Detecting a lying OS. Deleting `SyncPolicy::Lying`. Inventing a TCG guest.
- Restoring production WAL onto io_uring (R-uring). Extracting `db.rs`. `never_floor`.
- Rocks coluna A/B. crates.io.
