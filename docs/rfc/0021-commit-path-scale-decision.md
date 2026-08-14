# RFC-0021 P2.2 — Commit path scale decision (numbers-backed)

**Status:** draft decision record  
**Updated:** 2026-08-13  
**Parent:** [0021-montanha-fdb-tikv-parity-gaps.md](0021-montanha-fdb-tikv-parity-gaps.md)

## Options

| Option | Description |
|--------|-------------|
| **A** | Scale by **more multi-Raft ranges** on monolithic peer (Raft+Pedra same process) — TiKV-like |
| **B** | **Unbundle** log vs storage (FDB-like roles) |

## Decision (v0, reversible)

**Default: A** until P0.3/P1.6 benches show commit p99 or leader CPU cannot be fixed by range split + group commit on Pedra.

Rationale:
- Lab already multi-Raft; PD/split (P1.2) is the natural next scale lever.
- Unbundling (B) is higher cost and not justified without measured ceiling on A.
- Revisit after `findings/perf-*/perf_report.json` and soak artifacts exist on representative hardware.

## Required inputs before flipping to B

1. Perf gate v0 + soak with ≥N ranges showing p99 regression wall.  
2. Profile: time in Raft AE vs Pedra apply vs fsync.  
3. Explicit migration note if B is chosen.

## Non-decision

- Not claiming FDB unbundled topology.  
- Not forbidding B later.
