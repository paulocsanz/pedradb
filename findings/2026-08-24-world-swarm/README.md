# World swarm reference campaign — 2026-08-24 (RFC-0059)

Parallel World campaigns after the P0 fixes (CRC frame, escape-proof
discard, CommitUnknown payload check, stale-InstallSnapshot rejection)
and the checker hardening (union ground truth, per-range dual-claim
probe). All runs: buggify faults on, cross-node consistency checker on,
in-memory node backend (`WorldEnv` mem), 8 workers, 12 schedule steps.

| Shape | Seeds | Seed base | seeds/s | Failures |
|-------|-------|-----------|---------|----------|
| 3 nodes / 1 range | 16384 | 100000 | 4486.7 | **0** |
| 7 nodes / 1 range | 4096 | 200000 | 1623.0 | **0** |
| 9 nodes / 4 ranges | 4096 | 300000 | 475.9 | **0** |

Files: `<shape>/swarm.jsonl` (per-seed row: hash, puts_ok, oracles) and
`<shape>/swarm_summary.json`.

## What these campaigns found (fixed, pinned by regression tests)

- seeds 103906/104853/105654/106007/106008/106727/107007/112384 (3n):
  checker false-phantoms from a lagging single-reader ground truth
  (fixed: union of participating changelogs) **and** one real store bug —
  seed 104853: InstallSnapshot with `last_included_index` below the
  follower's commit wiped newer applied user state the retained log
  prefix never re-applies (`world_regression_seed104853_stale_snapshot_wipe`).
- seeds 304064/304074 (9n/4ranges): dual-claim oracle probed a key of an
  unrelated range while judging claims per range — legitimate Strong
  reads flagged as fail-open (fixed: per-range probe key; `Strong`
  already fails closed on the key's own range).

## Honest metrics

Throughput is machine-local (Apple silicon, 8 workers) and only meaningful
as exploration capacity: seeds/s × cores × budget. No CPU-hours-vs-FDB
claim. Determinism gate is per-seed `trace_hash` (serial-vs-parallel and
run-vs-run `cmp` of the JSONL).
