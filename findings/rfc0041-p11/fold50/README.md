# RFC-0041 — 50 ms fold multi-hold (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`fold50/run{1,2,3}`). JSONs only. Peers `sync: false`.

Hypothesis: parkfold2 fold during `active ≤ 1` still ran in apply_mc4
gaps (p95 2–3 ms). Hold fold for 50 ms after the last multi-writer
submit so MC never pays BTree absorb.

## Result

**Rejected.** apply_mc4 Pedra **4.3 k → 2.6–3.4 k**. Scan **2.21 →
~1.0** (run3 17 k, max 102 ms) — 50 ms hold left many unfolder
parked BTrees. Worker stays at 2 ms multi-hold (`11bd158`).

FLOOR not enabled.
