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

### Lab numbers (`montanha-fdb-bench` suite `scale`)

**S1 — sequential single client (N=16)**

| Ranges | keys/s |
|--------|--------|
| 1–8 | ~0.9–2.1 (flat) |

**S2 — 4 TCP clients, keys partitioned by range (N=16 total ok)**

| Ranges | keys/s | vs 1-range |
|--------|--------|------------|
| 1 | ~1.9 | 1× (hot leader) |
| 4 | ~5.2 | **~2.7×** |
| 8 | ~8.4 | **~4.3×** |

**Read:** sequential client cannot exercise multi-leader parallelism. **Multi-client multi-range proves option A**: more ranges ⇒ higher aggregate put QPS when writers fan out. Artifact: `findings/fdb-bench-scale-s2c/`. Option B (unbundle) still not justified.

Rationale:
- Lab multi-Raft scales write capacity under concurrent partitioned clients.
- Unbundling (B) is higher cost until multi-client multi-range saturates CPU with amortised fsync.
- Layers should shard by range prefix and open N writers (not one hot client).

## Required inputs before flipping to B

1. Perf gate v0 + soak with ≥N ranges showing p99 regression wall.  
2. Profile: time in Raft AE vs Pedra apply vs fsync.  
3. Explicit migration note if B is chosen.

## Non-decision

- Not claiming FDB unbundled topology.  
- Not forbidding B later.
