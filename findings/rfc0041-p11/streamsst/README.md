# RFC-0041 — stream L0 SST write (no collect/sort/body)

2026-08-17. `efdd52b`. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3.

Flush walks the memtable in InternalKey order, encodes into a reused
scratch, and writes header+data+index+bloom with incremental CRC32C.
No full-table `Vec` and no second 64 MiB `body` memcpy. G1 unchanged
(WAL still covers keys until rotate).

## Result

**3/16 ≥ 2.0** (C 2.46, E 2.40, MVCC 2.26). Do **not** enable
`ROCKS_PARITY_RATIO_FLOOR=2.0`.

apply_mc4 median **0.74** (Pedra 1.8 k) — same band as lazysst. Run1
apply 1c max still **224 ms**: lz4/write of the 64 MiB table (or rotate
`fdatasync` of that file) still shows up as a tail. YCSB A 1c **0.11**
(42 k, p50 28 µs) remains one WAL `fdatasync` per Ok.

| shape | lazysst | streamsst |
|---|---:|---:|
| ycsb_c | 1.81 | **2.46** |
| ycsb_e | 2.71 | **2.40** |
| deps_mvcc_latest | 2.61 | **2.26** |
| deps_apply_batch_mc4 | 0.79 | 0.74 |
| ycsb_a | 0.11 | 0.11 |
