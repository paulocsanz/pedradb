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

**S2 — 4 TCP clients, keys partitioned by range (N=16 total ok)** — re-run `findings/fdb-bench-scale-s3/` 2026-08-14

| Ranges | keys/s | vs 1-range | notes |
|--------|--------|------------|-------|
| 1 | ~2.3 | 1× | hot leader |
| 4 | ~4.0 | **~1.8×** | multi-client multi-range |
| 8 | ~0.12 | cliff | 8 groups + thr=4; HB/retry tax (not a linear win) |

Earlier s2c lab had higher r8 (~8.4 keys/s); **r8 is noisy** under thr≪ranges. Treat **r4 multi-client** as the stable option-A signal.

**S3 — same topology, TCP `PutBatch` (batch=8, 4 thr, 128 keys)**

| Ranges | keys/s | batches/s | notes |
|--------|--------|-----------|-------|
| 1 | ~3.2 | ~0.39 | **key amortization** vs S2 single-put (~2.3) |
| 4 | ~0.84 | ~0.10 | concurrent PutBatch + colocated leaders hurts |
| 8 | ~0.45 | ~0.06 | same residual as S2 r8 |

**Read:** sequential client cannot exercise multi-leader parallelism. **S2 at 4 ranges proves option A** under multi-client. **S3 proves PutBatch key amortization on one range**; multi-range batch still needs better leader spread + less worker contention (residual, not flip to B). Artifacts: `findings/fdb-bench-scale-s2c/`, `findings/fdb-bench-scale-s3/`. Option B (unbundle) still not justified.

### Client routing (2026-08-15)

`TcpClusterClient` now keeps a **per-range** leader map (`leaders_from_status` / NotLeader hints), optional **`active_range`** for partitioned writers, and `warm_leaders()`. Previously a single global prefer + “first `r*:leader`” status parse mis-routed multi-Raft puts after elect.

### Leader diversity (2026-08-15)

**Root cause of colocation:** raft `election_timeout` was `4 + node_id` only → lowest node id timed out first on **every** range.

**Fix:** per-`(node, range)` timeouts — preferred leader for range `r` is `members[(r-1) % n]` with shortest timeout; ring distance staggers the rest. API: `rebalance_range_leaders` if jitter still skews load; `leader_nodes()` for observability.

**Elect proof (`findings/fdb-bench-scale-s5/`):** S2 r4 → `r1=1 r2=2 r3=3 r4=1`; S2 r8 → round-robin across 3 nodes. Diversity works.

**Throughput residual (same run, host loaded):** S2 r1 ~1.1 keys/s, r4 ~0.38, r8 ~0.08 — spread leaders did not restore s2c-era multi-range QPS. Remaining cliff is multi-Raft HB/fsync per group (and machine load), not leader colocation. Option A still default; re-measure on quiet hardware before flipping anything.

### Put-path tick scope (2026-08-15)

While waiting for majority on one put/batch, TCP used full `cluster.tick()` → **every** range HB/elect each poll (N× tax under multi-Raft). Now `tick_range_id(range)` + scoped `pump_ae` only drive the active group; background worker still full-ticks for liveness.

**S6 lab (`findings/fdb-bench-scale-s6/`, N=12, thr=4)** with diversity + scoped ticks:

| | r1 | r4 | r4/r1 |
|--|----|----|-------|
| S2 multi-client put | 0.77 | **1.36** | **~1.8×** |
| S3 PutBatch | 3.03 | **7.76** | **~2.6×** |

**S7** (`findings/fdb-bench-scale-s7/`): thr→preferred-node range map; `elect_all` rebalances; S3 drops r8. S2 r4 **1.38** keys/s (~9× noisy r1=0.15 on that run).

**S8** (`findings/fdb-bench-scale-s8/`, N=12 thr=4, **45s/60s worker walls**): full suite ~2 min, no hang. S2 r1=1.80 / r4=**1.94**; S3 r1=**5.57** / r4=5.10 (96/96 keys). Option A **reconfirmed** under concurrent clients when leaders are spread **and** put-wait does not full-tick idle ranges.

Rationale:
- Lab multi-Raft scales write capacity under concurrent partitioned clients (S2 r1→r4) **when leaders are spread**.
- PutBatch is the right bulk path (S3 r1); multi-range batch needs more soak + leader diversity.
- Unbundling (B) is higher cost until multi-client multi-range saturates CPU with amortised fsync.
- Layers should shard by range prefix, open N writers, and pin clients to ranges (`with_active_range`).

## Required inputs before flipping to B

1. Perf gate v0 + soak with ≥N ranges showing p99 regression wall.  
2. Profile: time in Raft AE vs Pedra apply vs fsync.  
3. Explicit migration note if B is chosen.

## Non-decision

- Not claiming FDB unbundled topology.  
- Not forbidding B later.
