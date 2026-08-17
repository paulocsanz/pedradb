# RFC-0041 — 2 ms adaptive idle compact (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`wake2/run{1,2,3}`). JSONs only. Peers `sync: false`.

Hypothesis: okidle waited a full 5 ms after last Ok before L0 rewrite,
so compact started inside/after MVCC (L0=20–22 at scan). Worker now
waits 2 ms (`writes_until_idle`) so rewrite starts in the MVCC window.

## Result

**0/16 ≥ 2.0.** Compact of ~20 L0s does **not** finish before scan.

| shape | scansst | okidle | wake2 med |
|---|---:|---:|---:|
| deps_mvcc_latest | **2.580** | 1.31 | 1.098 |
| deps_scan | **1.152** | 0.263 | **0.205** |
| apply_mc4 | 0.956 | 1.128 | 1.484 |
| ycsb_c | 1.517 | — | 1.750 |
| ycsb_e | 0.913 | — | 1.002 |
| ycsb_a | 0.099 | 0.069 | 0.084 |

Probes: run1 L0=23/24 `scan_sst_probed=14562`; run2 L0=12/13
`10858`; run3 L0=21/22 `13764`. Same pile as okidle. apply_mc4
**1.484** is a weak Rocks (run3 2.2 k), not a product win.

**Rejected.** Starting compact 2 ms earlier does not rewrite 20 files
in a 4 ms MVCC + ~50 ms scan window. Next: park without SST during
writes; fold pairwise into one BTree; materialize only after a long
idle. FLOOR not enabled. 1c A still one WAL `fdatasync` per Ok.
