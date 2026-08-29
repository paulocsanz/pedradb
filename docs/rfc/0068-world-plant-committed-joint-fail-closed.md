# RFC: 0068 — World plant committed-joint-without-leave fail-closed

**Status:** in-progress
**Updated:** 2026-08-27
**Parents:** [0066](0066-joint-leave-fail-closed.md), [0064](0064-joint-election-fail-closed.md), [0063](0063-fdb-reliability-close-the-system-gap.md), [0051](0051-beyond-fdb-sim-holes.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig mid-run** (G7 operator change is outside `fdbserver -r simulation`). RFC-0066 P0 closed leave-joint on the store path. The remaining hole is the World schedule: a crash window (joint committed, apply lag, auto-leave skipped) was only planted by poking private log fields in a unit test, not as a World `Action` on the live cluster.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. It names one Sim2-class hole: DST can plant committed C-old,new without leave; election must still require C-new. Campaign, not a theorem of all reconfigs.

## Background

- RFC-0066: `pending_joint_on` keeps committed C-old,new until a leave (`old == new`) commits. Store test `election_after_committed_joint_still_requires_new_majority` plants by mutating `RangePeer` fields.
- World already has `JointAdd` / `JointRemove` (those call `leave_joint` after append). It had no action that plants the 0066 crash window.
- FDB Sim2 does not reconfig the coordinator mid-run. Pedra World can — this slice puts the tooth on that path.

## Problems This Solves

- **Problem:** the 0066 tooth lived only in a store unit test that pokes `p.log` / `p.commit` / `p.applied`. World fingerprints never saw committed-without-leave.
- **Problem:** `R-joint` close-text still said “Stateright leave + World plant = 0066 P1”.
- **Problem:** AS-IS `joint_still_active_as_is` would elect on C-old after that plant.

## Proposed Solution

- Production `StoreCluster::plant_committed_joint_without_leave` (DST seam, same class as BitFlip): committed `MembershipJoint`, apply lag, no leave.
- `probe_old_majority_joint_election` drives the live `election_has_joint_quorum` vote map.
- World `Action::PlantCommittedJoint` records `silent_wrong` if C-old majority elects.

## Delivery slices (mandatory)

### P0 — must ship first (plant on the live store/World path)
- [x] **P0.1** `plant_committed_joint_without_leave` + `probe_old_majority_joint_election` on `StoreCluster` — status: `done`
- [x] **P0.2** World `Action::PlantCommittedJoint` (explicit schedules only; not in `schedule_from_seed`) — status: `done`
- [x] **P0.3** Regression: plant then C-old majority does not elect; AS-IS would — status: `done` (`plant_committed_joint_without_leave_refuses_old_majority`, `world_planted_committed_joint_old_majority_does_not_elect`)

### P1 — next wave
- [ ] **P1.1** Stateright model includes leave-joint (0066 P1.1) — status: `todo`
- [ ] **P1.2** Random scheduler may emit PlantCommittedJoint (fingerprint bump, opt-in) — status: `todo`

### P2 — later
- [x] **P2.1** Verus twin of `joint_still_active` (0066 P2.1) — status: `done` (RFC-0095)
- [ ] **P2.2** L28 REAL: plant committed-without-leave on `cluster_real` — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | plant + probe on StoreCluster | done | plant_committed_joint_without_leave | 2026-08-27 |
| P0.2 | p0 | World Action PlantCommittedJoint | done | schedule.rs + World::run | 2026-08-27 |
| P0.3 | p0 | old-majority tooth after plant | done | plant_…_refuses_old_majority + world_planted_… | 2026-08-27 |
| P1.1 | p1 | Stateright leave-joint | todo | — | 2026-08-27 |
| P1.2 | p1 | scheduler emits plant | todo | — | 2026-08-27 |
| P2.1 | p2 | Verus joint_still_active | done | RFC-0095 | 2026-08-28 |
| P2.2 | p2 | L28 REAL plant | todo | — | 2026-08-27 |

## Acceptance Criteria

- **Tests**
  - `plant_committed_joint_without_leave_refuses_old_majority`: open 4, elect, joint-remove 4, plant add-4 without leave, `probe_old_majority_joint_election(1)` is false; `joint_still_active_as_is` is false (AS-IS would drop the joint); `joint_election_ok_as_is(2,3,Some((2,4)))` is true.
  - `world_planted_committed_joint_old_majority_does_not_elect`: World Queued schedule JointRemove then PlantCommittedJoint; `silent_wrong==0`; event `joint_plant_old_refused`.
  - Existing `election_after_committed_joint_still_requires_new_majority` stays green.
- **Telemetry / Analytics:** none — safety invariant. World `silent_wrong` if the tooth fails.
- **Documentation:** this RFC; `residuals.json` `R-joint` close-text; 0066 P1.2 done.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Extracting `db.rs` (L46). Flow (L29). Proving Linux/`fsync`/CPU/`rustc`.
- Closing `never_floor`. RFC-0067 Direct pin. Rocks coluna A/B. crates.io.
- Putting PlantCommittedJoint on the default seed scheduler (would bump fingerprints).
