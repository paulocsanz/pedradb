# RFC-0041 — notify worker only when imm staged (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`nowake/run{1,2,3}`). JSONs only. Peers `sync: false`.

Hypothesis: per-write `try_send(Run)` woke the host on every apply_mc4
batch and contended for the write lock. Wake only when `has_imm()`.

## Result

**Rejected.** apply_mc4 Pedra **4.3 k → 0.7–3.4 k** (run3 wall 11 s,
max 399 ms). Delayed park let the live mem grow past 4 MiB. Scan/E
also noisy (E run3 0.96). Restore per-write notify; fold only on the
5 ms poll, not on `Run`.

FLOOR not enabled.
