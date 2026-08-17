# RFC-0041 — park on Run, fold only on 5 ms poll (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`parkonly/run{1,2,3}`). JSONs only. Peers `sync: false`.

Hypothesis: per-write `Run` should only `park_imm_once`; fold on Timeout
so apply_mc4 does not absorb BTrees.

## Result

**Rejected.** apply_mc4 Pedra **2.2–2.5 k** (parkfold2 was 4.3 k). Scan
**66–151 k** (parkfold2 418 k) — less fold during 1c left many parked
BTrees. Worker restored to park+fold on both Run and Timeout.

FLOOR not enabled.
