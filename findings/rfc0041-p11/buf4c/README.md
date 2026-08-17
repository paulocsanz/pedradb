# RFC-0041 — 4 MiB + compact-at-trigger during writes (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3.

Hypothesis: 4 MiB flush left L0=23 after apply (buf4); one L0→L1 job
per worker tick when `L0 ≥ 4` would keep L0 near the trigger without
drain-to-0.

## Result

**0/16 ≥ 2.0.** L0 at MVCC: 0–3 (fix worked). Apply paid the rewrite:

| shape | buf4 | buf4c | Pedra qps |
|---|---:|---:|---:|
| apply 1c | **1.245** | 0.358 | 2 733 → 1 430 |
| apply_mc4 | **1.167** | 1.038 | 3 125 → 2 287 |
| ycsb_e | 1.916 | 1.662 | |
| deps_mvcc_latest | **2.082** | 1.566 | |

Same class as drain-to-0 during apply (3.5 k→1 k). Reverted. Ship
buf4: 4 MiB + idle persist + idle compact only.
