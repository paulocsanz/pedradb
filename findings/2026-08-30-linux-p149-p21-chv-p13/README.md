# RFC-0154 P1.3 — empty-prefix HashMap, CHV **REFUSED**

**When:** 2026-08-30T01:02:21Z–01:10:17Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Hypothesis:** HashMap on empty prefix (YCSB/kvrocks point) would push
a/d/set/cache over 3× without hurting scan (`cached_point_ord`).  
**Result:** **6/17 FAIL** min **0.458**. Worse than P1.2 BTree-empty
**7/17** min 1.171. Code **reverted**.

| shape | P1.2 (empty BTree) med | P1.3 (empty HashMap) med | Δ |
|---|---:|---:|---|
| ycsb_c | **3.23** | 2.996 | lost 3× |
| ycsb_e | **15.5** | **4.01** (min 2.37) | scan tax of HashMap sort |
| ycsb_a | 2.53 | 2.81 | small up, still <3 |
| kvrocks_set | 2.45 | 2.64 | small up, still <3 |
| deps_raftlog | 1.29 | 0.98 (min **0.46**) | worse |
| deps_mvcc_latest | **3.48** | **3.93** | still >3 |
| kvrocks_scan | **53** | **55** | still >3 |

`RESULT=P149_FAIL over_med=6/17 min_ratio=0.458`

Empty prefix is both **point** (c/get/set) and **range** (e, kvrocks_scan).
HashMap helped 1c put a little and taxed ordered scan enough to drop c
and smash e's min. Not the 2 extra shapes. Keep empty prefix as BTree
(P1.2 tree). Serial: `serial.log`.
