# RFC-0021 P2.2 — Commit path scale decision (numbers-backed)

**Status:** decision confirmed (lab 2026-08-14)  
**Updated:** 2026-08-14  
**Parent:** [0021-montanha-fdb-tikv-parity-gaps.md](0021-montanha-fdb-tikv-parity-gaps.md) · [0025](0025-montanha-perf-parity-vs-peers.md)

## Options

| Option | Description |
|--------|-------------|
| **A** | Scale by **more multi-Raft ranges** on monolithic peer (Raft+Pedra same process) — TiKV-like |
| **B** | **Unbundle** log vs storage (FDB-like roles) |

## Decision (v0, reversible)

**Default: A** (more multi-Raft ranges on monolithic peer). Reconfirmed 2026-08-14.

### Lab numbers (`montanha-fdb-bench` suite `scale`, N=16 sequential client)

| Ranges | keys/s (disjoint put) |
|--------|------------------------|
| 1 | ~1.7 |
| 2 | ~1.9 |
| 4 | ~2.2 |
| 8 | ~1.5 |

**Read:** single-threaded client does **not** get linear speedup from more ranges (serialized client + per-put majority still dominate). Option A still wins for **multi-writer** (one leader per range) when layers fan out concurrent clients — measure that next before B. Artifact: `findings/fdb-bench-scale/`.

Rationale:
- Lab already multi-Raft; PD/split is the natural next scale lever.
- Unbundling (B) is higher cost and not justified while sequential put is fsync/Raft limited.
- Revisit B only if multi-client multi-range saturates leader CPU with healthy fsync amortisation.

## Required inputs before flipping to B

1. Perf gate v0 + soak with ≥N ranges showing p99 regression wall.  
2. Profile: time in Raft AE vs Pedra apply vs fsync.  
3. Explicit migration note if B is chosen.

## Non-decision

- Not claiming FDB unbundled topology.  
- Not forbidding B later.
