# RFC: 0148 — Out-of-band remove_member forgets sent_through

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0147](0147-drop-repl-slot.md), [0143](0143-discard-uncommitted-local-non-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. Out-of-band `remove_member` already drops `next_index`/`match_index` for the removed id on remaining members. It does **not** drop `sent_through`. Discard’s escaped check scans every `sent_through` value, so a leftover in-flight index of the ex-member can block a later uncommitted discard. AS-IS `drop_sent_through` is false. This slice: oob remove forgets `sent_through` of a peer not in `ids`. 0147 joint `drop_repl_slot` is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Discard’s escaped check scans every `sent_through` value across `nodes`.
- oob remove already drops next/match; `sent_through` stays.

## Problems This Solves

- **Problem:** leftover `sent_through` of a removed peer survives oob `remove_member`.
- **Problem:** that leftover can block live discard of a later uncommitted index.
- **Problem:** AS-IS keep `sent_through`.

## Proposed Solution

- Pure `drop_sent_through(in_ids)` = `!in_ids`. AS-IS false. `remove_member` drops `sent_through` for the removed id on remaining members. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (oob remove)
- [x] **P0.1** `drop_sent_through` + oob drop — status: `done`
- [x] **P0.2** Regression — status: `done` (`remove_drops_removed_sent_through`)

### P1 — next wave
- [x] **P1.1** Live replication then oob remove drops sent_through — status: `done` (`oob_remove_drops_live_sent_through`)
- [ ] **P1.2** TCP 3-process — status: `todo`

### P2 — later
- [x] **P2.1** Verus twin + catalog pair — status: `done`
- [x] **P2.2** Campaign is not ∀ traces; R-verus still never — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | drop sent_through on oob remove | done | remove_member | 2026-08-28 |
| P0.2 | p0 | planted sent_through is gone | done | remove_drops_removed_sent_through | 2026-08-28 |
| P1.1 | p1 | live sent_through drops | done | oob_remove_drops_live_sent_through | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | todo | — | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs + drop_sent_through | 2026-08-28 |
| P2.2 | p2 | not ∀ traces / R-verus | done | drop_sent_through_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `drop_sent_through(false)` true; AS-IS false. Same tokens raft=store.
  - `remove_drops_removed_sent_through`: lab 3-node (oob 4→3 hits the quorum floor); plant `sent_through` for 3 on node 1; `remove_member(3)`; key 3 gone, remaining-member key stays. next/match already dropped and 0147 joint install are **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.1 `oob_remove_drops_live_sent_through`: after live replicate, oob remove, remaining peers have no `sent_through` for 3. plant is **not** this tooth.
  - P2.1 catalog pair `drop_sent_through` `entry: drop_sent_through`; twin `membership_joint.rs`; freeze twins fail if the exec fn is dropped. Does not run `verus`.
  - P2.2 `drop_sent_through_campaign_is_not_forall_traces`: `R-joint` stays continuous; campaign not a theorem. `drop_sent_through_verus_still_never`: `never_floor` still lists `R-verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0147 P1.2 stays TCP 3-process; `residuals.json` `R-joint` owner 0148.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
- Clearing the removed node’s own `sent_through` map (discard still scans `nodes`; later).
