# RFC-0041 — drain L0 during writes; scan via SST not retired BTrees

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`scansst/run{1,2,3}`). JSONs only. Peers `sync: false`.

Worker drains imm → L0 during writes (retire2 apply/MVCC). Scan/count
merge **all L0 files**, not the retired BTree chain (retire2 scan 0.27
was that merge). Fold-every-tick, park dump-all, and 1-L0/tick mid-apply
stay rejected. Park-without-SST API kept for tests / later idle policy.

## Result

**1/16 ≥ 2.0** (`deps_mvcc_latest` **2.580**). FLOOR not enabled.

| shape | retire2 | foldidle | incrfold | scansst med | Pedra qps |
|---|---:|---:|---:|---:|---:|
| deps_mvcc_latest | **3.08** | 1.31 | 2.04 | **2.580** | 308 588 |
| deps_scan | 0.27 | 0.68 | 0.94 | **1.152** | 87 740 |
| ycsb_c | 1.06 | 1.47 | 1.73 | 1.517 | 1 688 833 |
| ycsb_e | 1.63 | **2.29** | 1.64 | 0.913 | 89 449 |
| apply_mc4 | **1.45** | 1.25 | 0.67 | 0.956 | 2 070 |
| ycsb_a | 0.09 | 0.09 | 0.09 | 0.099 | 29 050 |

1c A/F still one WAL `fdatasync` per Ok (p50 ~31 µs). G1 unchanged.
