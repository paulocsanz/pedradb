# RFC: 0147 — Forget replication slots of a removed node

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0146](0146-hint-if-member.md), [0143](0143-discard-uncommitted-local-non-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. Out-of-band `remove_member` drops `next_index`/`match_index` for the removed id. Log-carried joint leave via `install_applied_membership` does not. Remaining leaders keep `sent_through`/`next_index` for the ex-member. That made 0143 tests have to clear `sent_through` so discard would not treat a new uncommitted index as already escaped. AS-IS `drop_repl_slot` is false. This slice: C-new apply forgets slots whose peer is not in `ids`. 0146 hint filter is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Discard’s escaped check scans every `sent_through` value.
- oob remove drops next/match; joint leave does not.

## Problems This Solves

- **Problem:** leftover `sent_through` of a removed peer can block live discard.
- **Problem:** leftover next/match of a removed peer survive C-new apply.
- **Problem:** AS-IS keep those slots.

## Proposed Solution

- Pure `drop_repl_slot(in_ids)` = `!in_ids`. AS-IS false. `install_applied_membership` drops next/match/sent_through for non-members. AE/snapshot replies from a non-member do not re-insert. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (C-new apply)
- [x] **P0.1** `drop_repl_slot` + slot drop — status: `done`
- [x] **P0.2** Regression — status: `done` (`install_drops_removed_repl_slots`)

### P1 — next wave
- [x] **P1.1** After queued leave, remaining peers have no slot for 4 — status: `done` (`leave_drops_removed_repl_slots`)
- [ ] **P1.2** TCP 3-process — status: `todo`

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | drop slots on C-new | done | install_applied_membership | 2026-08-28 |
| P0.2 | p0 | planted slot is gone | done | install_drops_removed_repl_slots | 2026-08-28 |
| P1.1 | p1 | leave itself drops slots | done | leave_drops_removed_repl_slots | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | todo | — | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + drop_repl_slot | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | drop_repl_slot_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `drop_repl_slot(false)` true; AS-IS false. Same tokens raft=store.
  - `install_drops_removed_repl_slots`: Queued 4→3 leave; plant `next_index`/`match_index`/`sent_through` for 4 on node 1; `install_applied_membership(C-new)`; those keys gone. 0146 hint clear is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `leave_drops_removed_repl_slots`: after leave, remaining local peers have no repl slot for 4. plant+re-install is **not** this tooth.
  - P2.1 catalog pair `drop_repl_slot` `entry: drop_repl_slot`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `drop_repl_slot_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `drop_repl_slot_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0146 P1.2 stays TCP 3-process; `residuals.json` `R-joint` owner 0147.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
  - oob `remove_member` already drops next/match; `sent_through` there is later.
