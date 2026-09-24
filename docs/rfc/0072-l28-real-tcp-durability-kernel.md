# RFC: 0072 — L28 REAL TCP durability kernel (get ∧ after-kill ∧ restart)

**Status:** done
**Updated:** 2026-08-27
**Parents:** [0064](0064-joint-election-fail-closed.md), [0063](0063-fdb-reliability-close-the-system-gap.md), [0051](0051-beyond-fdb-sim-holes.md)

**Residual:** `R-swarm-real` (not `never_floor`). Axis vs FDB Sim2: **G4 real TCP / multi-process**. Sim2 is `Sim2Conn` (`deque` + `delay()`); Pedra `cluster_real` is 3 OS processes of `montanha-tcp`. The durability oracle (acked put survives SIGKILL + reopen) lived as string-contains glue in `cluster_real::main`. AS-IS treats a successful first `get` as enough (ignores kill/restart). This slice names the gate on the production bin path.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Two-seed fingerprint replay stays a campaign, not a theorem of all TCP schedules.

## Background

- RFC-0063/0064: `cluster_real` + `l28_real_tcp_seed_replay`. Fingerprint is outcomes (`put/get/after/restart`), not World `trace_hash` (wall-tick elect).
- `main` exited 1 if the line lacked `get=1`, `after=1`, `restart=1` — inline, not a kernel the store tests call.
- FDB G4: real TCP is outside Sim2. Pedra has a REAL cluster bin; the pass/fail rule must be a named decision.

## Problems This Solves

- **Problem:** L28 “clean” can be rounded to “first get succeeded.”
- **Problem:** the durability triple lived only in the bin’s string parse.
- **Problem:** AS-IS admits get-only (kill/restart ignored).

## Proposed Solution

- Pure `l28_durability_ok(get, after_kill, restart)` = all three. AS-IS = `get` only.
- Production `cluster_real` calls the kernel (not string contains).
- In-process StoreCluster test drives open/elect/put/crash-reopen then the same kernel.

## Delivery slices (mandatory)

### P0 — must ship first (gate on the live cluster_real / store path)
- [x] **P0.1** `l28_durability_ok` + AS-IS — status: `done`
- [x] **P0.2** `cluster_real` exits via the kernel — status: `done`
- [x] **P0.3** Regression: in-process put + crash-reopen; kernel requires after∧restart; AS-IS would pass on get-only — status: `done` (`l28_durability_ok_requires_after_kill_and_restart`)

### P1 — next wave
- [x] **P1.1** `l28_real_tcp` asserts via the kernel (not only `assert_eq` of lines) — status: `done` (`l28_durability_ok` on replay + leader-kill)
- [x] **P1.2** World `silent_wrong` seed must pass `cluster_real --seed S` — status: `done` (`world_seed_l28_ok`; seed `0x641e28`; `world_l28_seed_silent_wrong_zero_is_not_tcp_clean_alone`)

### P2 — later
- [x] **P2.1** Catalog pair + Verus twin — status: `done` (payment now the Aeneas extract of the linked kernel `src/l28.rs` — `scripts/aeneas_l28.sh`; the `verus/l28.rs` mirror twin was deleted, catalog `l28_durability`)
- [x] **P2.2** Leader-kill path named in the kernel (already a bool in the bin) — status: `done` (`l28_leader_kill_ok`; `cluster_real --leader-kill`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | l28_durability_ok kernel + AS-IS | done | l28.rs | 2026-08-27 |
| P0.2 | p0 | cluster_real calls kernel | done | cluster_real.rs | 2026-08-27 |
| P0.3 | p0 | after-kill ∧ restart tooth | done | l28_durability_ok_requires_after_kill_and_restart | 2026-08-27 |
| P1.1 | p1 | l28_real_tcp uses kernel | done | l28_real_tcp_seed_replay | 2026-08-27 |
| P1.2 | p1 | World silent_wrong → cluster_real | done | world_seed_l28_ok + seed 0x641e28 | 2026-08-27 |
| P2.1 | p2 | catalog + Aeneas extract | done | aeneas_l28.sh over src/l28.rs (mirror twin deleted) + catalog l28_durability | 2026-08-27 |
| P2.2 | p2 | leader-kill in kernel | done | l28_leader_kill_ok + cluster_real --leader-kill | 2026-08-27 |

## Acceptance Criteria

- **Tests**
  - `l28_durability_ok(true,true,true)` true; missing after or restart → false; AS-IS true on get-only.
  - `l28_durability_ok_requires_after_kill_and_restart`: `StoreCluster::open`, `elect_all`, `put`, `get_strong`, `crash_reopen_engine_on` a follower; kernel true on the three live observations.
  - P1.1 `l28_real_tcp_seed_replay` / `l28_real_tcp_leader_kill` assert via `l28_durability_ok` (not only line `contains`).
  - P1.2 `world_seed_l28_ok(0, false)` is false (AS-IS would pass); World seed `0x641e28` has `silent_wrong=0` (`world_l28_seed_silent_wrong_zero_is_not_tcp_clean_alone`) and the same seed’s `cluster_real` is L28-clean.
  - P2.2 `l28_leader_kill_ok` requires get∧after∧restart; AS-IS get-only. `cluster_real --leader-kill` and `l28_real_tcp_leader_kill` call the named gate.
  - P2.1 catalog pair `l28_durability` entry `l28_durability_ok` paid by the Aeneas extract of `crates/pedradb-store/src/l28_kernel.rs` (single artifact; `verus/l28.rs` twin deleted).
- **Telemetry / Analytics:** none — durability invariant. `cluster_real` still prints the fingerprint line.
- **Documentation:** this RFC; `residuals.json` `R-swarm-real` close-text + owner 0072.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Proving all TCP schedules. Wall-clock elect seed-stability (G8).
- Extracting `db.rs`. Flow. `never_floor`. Rocks coluna A/B. crates.io.
- New `*_kernel.rs` (TCB freeze). Module is `l28.rs`.
