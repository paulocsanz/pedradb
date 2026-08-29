# RFC: 0124 — Durable membership must override CLI/`--peer` on open

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0123](0123-verus-queued-leave-finish.md), [0066](0066-joint-leave-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. After a leave apply, `persist_cluster_identity` writes C-new to disk. `bind_cluster_identity` **decodes** that key then always `persist_cluster_identity` of **RAM** `ids` (TCP: CLI `--peer`). A process restart with the original peer list **overwrites** C-new with C-old. AS-IS `disk_membership_overrides_cli` is false. This slice: if disk membership is non-empty, it is `ids`. 3-process on-disk leave scan is **not** this tooth (0123 P1.2).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `install_applied_membership` persists C-new.
- `open_single_node` / `montanha-tcp` pass CLI member ids; `bind_cluster_identity` ignores disk voters.

## Problems This Solves

- **Problem:** restart with stale `--peer` resurrects removed voters.
- **Problem:** 0123 crash-reopen keeps RAM `ids`, not disk.
- **Problem:** AS-IS CLI wins.

## Proposed Solution

- Pure `disk_membership_overrides_cli(has_disk)` = `has_disk`. AS-IS false. `bind_cluster_identity` restores `ids` from disk before persist. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (disk voters win)
- [x] **P0.1** `disk_membership_overrides_cli` + bind restore — status: `done`
- [x] **P0.2** Regression — status: `done` (`disk_membership_overrides_cli_after_leave`)

### P1 — next wave
- [x] **P1.1** Persist identity **before** applied on MembershipJoint — status: `done`
- [x] **P1.2** `crash_reopen_engine_on` reloads disk membership — status: `done` (`crash_reopen_reloads_disk_membership`)

### P2 — later
- [x] **P2.1** Verus twin — status: `done` (catalog `disk_membership` / `disk_membership_overrides_cli`)
- [x] **P2.2** Campaign is not ∀ traces — status: `done` (`disk_membership_overrides_cli_campaign_is_not_forall_traces`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | disk membership restores ids | done | bind_cluster_identity | 2026-08-28 |
| P0.2 | p0 | leave then stale CLI | done | disk_membership_overrides_cli_after_leave | 2026-08-28 |
| P1.1 | p1 | identity before applied | done | apply_range | 2026-08-28 |
| P1.2 | p1 | crash_reopen reloads voters | done | crash_reopen_reloads_disk_membership | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | catalog disk_membership + membership_joint.rs | 2026-08-28 |
| P2.2 | p2 | not ∀ traces | done | disk_membership_overrides_cli_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `disk_membership_overrides_cli(true)` true; AS-IS false.
  - `disk_membership_overrides_cli_after_leave`: Queued shrink + finish leave; disk membership omits 4; RAM `ids` reset to include 4; `bind_cluster_identity` restores disk; `!is_member(4)`. 0123 RAM crash-reopen is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 catalog pair `disk_membership` `entry: disk_membership_overrides_cli`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `disk_membership_overrides_cli_campaign_is_not_forall_traces`: `R-joint` stays continuous; catalog pair `disk_membership`; campaign not a theorem.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0123 P2 stays campaign/R-verus; `residuals.json` `R-joint` owner 0124.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
