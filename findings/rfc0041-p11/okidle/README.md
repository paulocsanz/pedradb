# RFC-0041 — idle clock = last Ok, not submit start

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`okidle/run{1,2,3}`). JSONs only. Peers `sync: false`.
Isolated `fdatasync` p50 **23.9 µs**.

`writes_idle_for` now uses submit **return** time. A 5–14 ms apply
batch used to look idle the instant it returned (last submit was
already older than 5 ms), so the host started L0 rewrite between
apply_mc4 ops.

## Result

**0/16 ≥ 2.0.** apply_mc4 **1.128** (Pedra 2.1 k — same as scansst).
Scan **0.263** (L0=20–22 at MVCC/scan — compact now waits a full 5 ms
after the last Ok). MVCC 1.31. 1c A **0.069** (21 k / 1/t_fd).

Semantics kept. FLOOR not enabled. P1.2 1c write 2× remains above
`1/t_fd` under G1; target and peer unchanged.
