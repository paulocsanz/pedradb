# RFC: 0130 — Recover must apply a committed unapplied prefix

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0129](0129-verus-identity-before-applied.md), [0124](0124-disk-membership-overrides-cli.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. RFC-0124 restores disk voters and persists identity **before** applied. `crash_reopen_engine_on` / `recover_after_open` load `commit` and `applied` then **never** call `apply_range`. A `MembershipJoint` that is majority-committed and durable in the log, but not yet applied, stays C-old across process death. AS-IS `recover_must_apply` is false. This slice: if `commit > applied`, recover applies. 0124 disk-membership restore of an **already applied** joint is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `load_range_peer` caps `applied ≤ commit` and keeps the committed log.
- `apply_range` is the only path that installs `MembershipJoint` into `ids`.
- Open / crash-reopen never walk that path.

## Problems This Solves

- **Problem:** crash between persist-commit and apply resurrects C-old.
- **Problem:** 0124 only wins when identity was already durable.
- **Problem:** AS-IS skips recover apply.

## Proposed Solution

- Pure `recover_must_apply(applied, commit)` = `commit > applied`. AS-IS false. `recover_apply_committed` on open and both reopen paths. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (crash-reopen applies)
- [x] **P0.1** `recover_must_apply` + recover helper — status: `done`
- [x] **P0.2** Regression — status: `done` (`crash_reopen_applies_committed_unapplied_joint`)

### P1 — next wave
- [x] **P1.1** Process `open` applies — status: `done` (`open_applies_committed_unapplied_joint`)
- [x] **P1.2** TCP 3-process recover apply — status: `done` (`l28_real_tcp_recover_apply`)

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | recover apply helper | done | recover_apply_committed | 2026-08-28 |
| P0.2 | p0 | crash-reopen committed joint | done | crash_reopen_applies_committed_unapplied_joint | 2026-08-28 |
| P1.1 | p1 | process open apply | done | open_applies_committed_unapplied_joint | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_recover_apply | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + recover_apply | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | recover_must_apply_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `recover_must_apply(1, 2)` true; AS-IS false. Same tokens raft=store.
  - `crash_reopen_applies_committed_unapplied_joint`: elect 4; plant durable committed shrink joint **without** apply; `crash_reopen_engine_on` leader; `!is_member(4)` and applied ≥ commit. 0124 already-applied disk identity is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `open_applies_committed_unapplied_joint`: same plant; drop process; `StoreCluster::open`; `!is_member(4)`. crash-reopen is **not** this tooth.
  - P1.2 `l28_real_tcp_recover_apply`: seed `0x0130_1E28` twice with `--remove-member`; fingerprints match; `apply=1`; plant a durable committed-unapplied Noop on a remaining voter's Pedra dir then production TCP ctor closes `recover_must_apply`. Rewind-applied is **not** this tooth (compact may have dropped that entry). 0131 removed-replica recover is **not** this tooth. Exit via `l28_tcp_apply_ok`. AS-IS would skip apply. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `recover_apply` `entry: recover_must_apply`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `recover_must_apply_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `recover_must_apply_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0131 P1.2 is the removed-replica TCP tooth; `residuals.json` `R-joint` owner 0132.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
