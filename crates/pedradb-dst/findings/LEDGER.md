# LEDGER — engine/cluster REALs (in-tree)

**Status:** living  
**Scope:** `pedradb` engine + cluster (`pedradb-core`, `pedradb-store`, `pedradb-world` trials).  
**Not** a copy of the sibling campaign ledger (`determinismo/pedradb-dst/findings/LEDGER.md`, 213 F-ids) — only REALs with an in-tree repro live here.

## Process (RFC-0050 P1.4)

1. A **REAL** = invariant violation (silent wrong, dual-leader fail-open, false
   majority, torn/corrupt acceptance) reproduced by a **seed** or a named test
   in **this** checkout — no `../determinismo` required.
2. When the World CI (`world-determinism` job) or any in-tree soak finds
   `silent_wrong > 0` / `dual_leader_fail_open > 0` / `false_majority > 0`,
   the finding lands in this directory **in the same change** as the triage —
   with seed, `trace_hash`, config, and the shrunk schedule.
3. Every entry closes with: root cause, fix (or accepted-residual rationale),
   and the regression test/seed that fails without the fix.
4. Findings never get deleted; superseded ones get a `Status:` update.

## Entries

| ID | Title | Status | Test / seed |
|----|-------|--------|-------------|
| F47 | failed 2PC finish + heal/reopen majority-installs the TX | closed (2026-08-14) | `fail_after_mid_2pc_restores_preimage` |
| F49 | outstanding Queued proposes collide on durable `si_gen` | FIXED | `queued_double_propose_distinct_si_gens_survive_reopen` |

Files: [`F47-store-failingenv-2pc-reopen-majority.md`](F47-store-failingenv-2pc-reopen-majority.md),
[`F49-si-gen-collision-outstanding-proposes.md`](F49-si-gen-collision-outstanding-proposes.md).

## World telemetry (rolling)

World CI keeps `silent_wrong=0`, `dual_leader_fail_open=0`, `false_majority=0`
on the fixed smoke band (seeds `0..=7`, `0xC0FFEE`). The first REAL found by a
seed appends a row above with `hits/256` and `trace_hash` in the same effort
window (RFC-0056 item 12 continuity).
