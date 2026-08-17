# RFC-0041 — park L0 pins; fold off-lock when idle; epoch caches

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`foldidle/run{1,2,3}`). JSONs only.

Absorbing every flushed mem into one BTree **under the write lock**
(fold remesura) cut apply_mc4 to ~1 k. This slice:

- Drain only **parks** the pin (push).
- Host worker **folds** pending pins into one `MemTable` off the write
  lock after `writes_idle_for(5 ms)`.
- Point/prefix/count caches invalidate by **generation bump** (O(1)),
  not a map walk.

G1 unchanged. FLOOR not enabled.

## Result

**1/16 ≥ 2.0** (`ycsb_e` **2.291**).

| shape | retire2 | foldidle med | Pedra qps |
|---|---:|---:|---:|
| ycsb_e | 1.63 | **2.291** | 165 190 |
| ycsb_c | 1.06 | 1.471 | 1 499 298 |
| apply_mc4 | 1.45 | 1.249 | 2 573 |
| MVCC | 3.08 | 1.309 | 223 623 |
| scan | 0.27 | 0.684 | 76 663 |
| ycsb_a | 0.09 | 0.091 | 28 324 |

1c A/F still one WAL `fdatasync` per Ok. Absorb-on-write-lock rejected.
