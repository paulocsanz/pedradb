# RFC-0041 — L0 write without SST `fdatasync` until WAL rotate

2026-08-17. `2feb021`. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`implementer/lazysst/run{1,2,3}`). JSONs only.

## Contract

Flush writes L0 bytes **without** file `fdatasync` and does **not** publish
MANIFEST. `rotate_wal_now` fsyncs pending L0s, persists MANIFEST, then
replaces the WAL. Acked keys stay in the WAL until that point (G1).
Crash with a torn unsynced L0: orphan GC + WAL replay
(`unsynced_l0_torn_sst_recovers_from_wal`). After rotate, deleting the WAL
still sees the key (`flush_rotate_makes_sst_sufficient_without_wal`).

## Result

**2/16 ≥ 2.0** (`ycsb_e` 2.71, `deps_mvcc_latest` 2.61). Do **not** enable
`ROCKS_PARITY_RATIO_FLOOR=2.0`.

apply_mc4 median **0.79** (Pedra 1.8 k) — same band as idle64. Skipping the
SST fd did not remove the apply tail (run1 max 182 ms): encoding/writing
the 64 MiB table still sits on the host worker. YCSB A 1c **0.11** (35 k,
p50 27 µs) — still one WAL `fdatasync` per Ok.

| shape | idle64 med | lazysst med |
|---|---:|---:|
| ycsb_c | 2.43 | 1.81 |
| ycsb_e | 4.26 | **2.71** |
| deps_scan | 18.0 | 1.65 |
| deps_mvcc_latest | 1.90 | **2.61** |
| deps_apply_batch_mc4 | 0.94 | 0.79 |
| ycsb_a | 0.16 | 0.11 |

Refuse-sync: compare vs a `sync: true` peer exits **2** (no allow-sync).
