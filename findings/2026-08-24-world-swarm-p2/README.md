# World swarm P2 campaign — 2026-08-24

Reference campaigns with RFC-0059 P2 knobs on:
`PEDRA_SWARM_UPGRADE=1 PEDRA_SWARM_TRAJECTORY=1`, 8 workers, 12 steps,
in-memory nodes, buggify + consistency on.

## Results

| Shape | Seeds | Base | Failures | wall | seeds/s |
|---|---|---|---|---|---|
| 3n / 1 range | 16384 | 400000 | **0** | 5.90s | 2775 |
| 7n / 1 range | 4096 | 500000 | **0** | 5.40s | 759 |
| 9n / 4 ranges | 4096 | 600000 | **0** | 24.77s | 165 |

JSONL + `swarm_summary.json` live next to this file (`3n/`, `7n/`, `9n-4ranges/`).

## Determinism gate

- Same-process serial vs 4-worker swarm: `world_swarm_parallel_matches_serial`
  and `world_swarm_parallel_matches_serial_membership_windows` — both green.
- Cross-process `cmp` of JSONL hashes is **not** bit-stable once membership
  windows are on (two `world_swarm` invocations of seed 500002 oscillate
  between two leader identities on a `put` that is already `not leader`).
  Oracles stay 0. The swarm contract is same-process (HashMap `RandomState`
  is per-process); do not treat cross-process hash equality as the gate.

## F-found (this campaign)

Pinned by seed; each is a product fix + unit mutant + World regression.

| Seed | Class | Fix |
|---|---|---|
| 500308 | `consistency_resurrected` — chained out-of-band removals shrank the voting set until a commit quorum of the shrunken config was disjoint from a later election quorum | Quorum floor in `StoreCluster::remove_member` (`2·(⌊m/2⌋+1) > high_water`) |
| 503976 | `consistency_resurrected` ×2 — three stacked defects | (1) election tally keyed `(range, term, candidate)` not `(range, term)`; (2) stale-snapshot reject is **failure + hint**, never a replication match; (3) InstallSnapshot labeled at the leader's **applied** point, not the compaction watermark |
| 502514 | `consistency_resurrected` — changelog-only snapshot wipe vs live `get` | Lazy CHANGELOG union is per-key (not `out.last().seq`); resurrection oracle requires the proving node's live get to be gone |

## How to replay

```bash
PEDRA_SWARM_UPGRADE=1 PEDRA_SWARM_TRAJECTORY=1 PEDRA_SWARM_LOG=dir \
  target/release/world_swarm 4096 500000 8 7 12
# single-seed forensic
PEDRA_SWARM_DUMP=503976 PEDRA_SWARM_UPGRADE=1 PEDRA_SWARM_TRAJECTORY=1 \
  target/release/world_swarm 1 503976 1 7 12
```
