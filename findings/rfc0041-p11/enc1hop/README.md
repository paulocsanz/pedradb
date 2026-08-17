# RFC-0041 — encode+append off the write lock, same hop count (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`enc1hop/run{1,2,3}`). JSONs only. Peers `sync: false`.

## Hypothesis

`encoff` (encode off lock + second absorb pass) cut apply_mc4 4.3 k → 1.7 k.
Retry with the **same lock-hop count** as today: prepare under the write lock,
encode+WAL-append off it, `fdatasync`, apply. No second absorb pass.

## Result

**Rejected. 0/16 ≥ 2.0 on a clean read of Pedra qps.** apply_mc4 Pedra
**654 / 1670 / 1841** (med **1670**) vs parkfold2 **4271**. YCSB A run1
**7921** (70 ms tails). Reverted to `group_start` + late-join absorb +
`finish_group_off_lock`. FLOOR off.

| shape | run1 | run2 | run3 | med ratio | Pedra med |
|---|---:|---:|---:|---:|---:|
| apply_mc4 | 0.586 | 1.030 | 1.805 | 1.030 | 1 670 |
| ycsb_a | 0.099 | 0.169 | 0.191 | 0.169 | 22 662 |
| ycsb_e | 1.850 | 1.886 | 2.852 | 1.886 | 154 719 |
| deps_scan | 0.860 | 3.099 | 3.213 | 3.099 | 181 528 |

G1 unchanged. Do not read run3 apply_mc4 1.81 as a win (Pedra 1.8 k, weak Rocks).
