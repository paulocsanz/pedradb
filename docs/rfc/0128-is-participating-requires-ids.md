# RFC: 0128 — `is_participating` must not count a non-member

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0127](0127-reopen-participating-follows-ids.md), [0105](0105-pending-joint-members-only.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G7 reconfig**. RFC-0127 fixes the flag on **reopen**. Live `is_participating` still returns `nodes[id].participating` when the node is in the map — a stale `true` after leave makes election/AE/compact helpers count a removed voter **without** crash-reopen. AS-IS `participating_if_member` is always true. This slice: `is_participating` calls the kernel on `ids.contains`. 0127 reopen insert is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `is_participating` prefers the flag over `ids`.
- `set_participating(false)` still in `ids` (partition) — must stay false.
- Removed from `ids` with stale flag true must be false.

## Problems This Solves

- **Problem:** leave+stale flag counts a removed node on the live path.
- **Problem:** 0127 only gated insert-on-reopen.
- **Problem:** AS-IS ignores `ids`.

## Proposed Solution

- `is_participating`: if kernel(`ids.contains`) then the flag (or `ids` when not in `nodes`), else false. Clone already has the fn. Verus twin + catalog pair. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (live path)
- [x] **P0.1** `is_participating` calls `participating_if_member` — status: `done`
- [x] **P0.2** Regression — status: `done` (`is_participating_ignores_stale_flag_after_leave`)

### P1 — next wave
- [x] **P1.1** Verus twin + catalog pair — status: `done`
- [x] **P1.2** TCP 3-process participating — status: `done` (`l28_real_tcp_participating_after_remove`)

### P2 — later
- [x] **P2.1** Campaign is not ∀ traces — status: `done` (`is_participating_campaign_is_not_forall_traces`)
- [x] **P2.2** R-verus still never — status: `done` (RFC-0129 `identity_before_applied_verus_still_never`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | live is_participating gated | done | is_participating | 2026-08-28 |
| P0.2 | p0 | stale flag after leave | done | is_participating_ignores_stale_flag_after_leave | 2026-08-28 |
| P1.1 | p1 | Verus twin | done | membership_joint.rs + catalog participating_member | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | l28_real_tcp_participating_after_remove | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | done | is_participating_campaign_is_not_forall_traces | 2026-08-28 |
| P2.2 | p2 | R-verus never | done | RFC-0129 | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `participating_if_member(false)` false; AS-IS true.
  - `is_participating_ignores_stale_flag_after_leave`: Queued 4→3 leave; force `participating=true` on 4; **no** crash-reopen; `!is_participating(4)` and `!is_member(4)`. 0127 reopen is **not** this tooth. `set_participating(false)` on a remaining member still reports false. Runs on Darwin. Does not submit io_uring SQEs.
  - P1.2 `l28_real_tcp_participating_after_remove`: seed `0x0128_1E28` twice with `--remove-member`; fingerprints match; `part=1`; TCP ctor with stale CLI `[1,2,3]` has `!is_participating(3)`. Exit via `l28_tcp_part_ok`. AS-IS would count a remote non-member. Runs on Darwin. Does not submit io_uring SQEs.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0127 P1.2 Verus done here; `residuals.json` `R-joint` owner 0128.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
