# RFC-0041 — 20 µs fat catch-up, 5 ms idle (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`fat20b/run{1,2,3}`). JSONs only. Peers `sync: false`.

avg_group **1.72–1.74** (was 1.54). apply_mc4 Pedra **1.47 k** (scansst
2.1 k). Extra wait costs more than the shared fd. **Rejected.** Catch-up
skip on fat batches restored. FLOOR not enabled.
