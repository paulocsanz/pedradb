# RFC-0041 — 20 µs fat catch-up + 1 ms idle compact (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`fat20/run{1,2,3}`). JSONs only. Peers `sync: false`.

1 ms idle + 2 ms poll started L0 rewrite in apply_mc4 gaps (p50 ~1.8
ms). apply_mc4 **0.557** (Pedra 1.2 k). avg_group only 1.57–1.75.

**Rejected** the 1 ms idle. Fat-batch 20 µs catch-up kept; idle back to
5 ms. FLOOR not enabled.
