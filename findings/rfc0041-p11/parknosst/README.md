# RFC-0041 — park imm without SST; dump-all on idle (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`parknosst/run{1,2,3}`). JSONs only. Peers `sync: false`.

During a write burst the host **parks** imm (move, no lz4). When writers
are idle 5 ms the worker materializes **every** parked table, persists,
rotates, and drains L0. Scan/count use L0 SSTs, not retired BTrees.

## Result

**0/16 ≥ 2.0.** Apply_mc4 held (**1.217**, Pedra 2.7 k) — no incrfold
regression — but idle dump landed in the MVCC/scan window (run1 MVCC
probe: L0=0 / L1=1 mid-shape). MVCC **2.04 → 1.07**. Scan 0.59.

| shape | incrfold | parknosst med | Pedra qps |
|---|---:|---:|---:|
| apply_mc4 | 0.67 | **1.217** | 2 672 |
| ycsb_c | 1.73 | 1.606 | 1 633 376 |
| ycsb_e | 1.64 | 1.644 | 115 305 |
| deps_mvcc_latest | **2.04** | 1.069 | 200 559 |
| deps_scan | 0.94 | 0.592 | 115 661 |
| ycsb_a | 0.09 | 0.091 | 27 666 |

**Rejected** as the idle policy. Next: at most **one** L0 per 5 ms tick
(writes or idle); compact only when parked is empty. G1 unchanged.
FLOOR not enabled.
