# RFC-0041 — drain writes + 1 compact of ≤4 L0s per idle tick (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`b4c1/run{1,2,3}`). JSONs only. Peers `sync: false`.

## Result

**1/16 ≥ 2.0** (MVCC **2.660**). Scan **0.276** (Pedra 32 k) — run1 probe
L0=**24**, `scan_sst_probed=14582`. One 4-file job per 5 ms cannot drain
the apply pile before scan. apply_mc4 1.17 (Pedra 2.0 k).

Rejected. Worker stays drain-during-writes + idle `while` compact
(scansst). Isolated `fdatasync` p50 **23.3 µs**. FLOOR not enabled.
