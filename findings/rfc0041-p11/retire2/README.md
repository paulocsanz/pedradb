# RFC-0041 — retired L0 memtables + first-hit MVCC

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`retire2/run{1,2,3}`). JSONs only.

After each L0 flush the memtable stays on the read path until that L0
is compacted. Scan/count skip the covering L0 SST. MVCC
`last_under_user_prefix` returns on the first (newest) mem layer.
Group WAL encode reuses scratch buffers. SST newest-first order is
cached. G1 unchanged (retired is a cache; WAL+SST remain the source).
WAL rotate still allowed with retired present.

## Result

**1/16 ≥ 2.0** (`deps_mvcc_latest` **3.083**). Do **not** enable FLOOR.

| shape | buf4 | retire2 med | Pedra qps |
|---|---:|---:|---:|
| deps_mvcc_latest | 2.08 | **3.083** | 547 570 |
| apply_mc4 | 1.17 | **1.451** | 2 572 |
| apply 1c | 1.25 | 0.654 | 2 716 |
| ycsb_e | 1.92 | 1.629 | 155 710 |
| ycsb_c | 1.11 | 1.056 | 1 141 878 |
| deps_scan | 0.50 | 0.272 | 60 146 |
| ycsb_a | 0.18 | 0.094 | 30 786 |

Run1/3 MVCC probe: `latest_sst_fallback=0`, `scan_sst_probed=0`.
Scan qps still dies on merging many retired BTrees (p50 is 0.4 µs;
qps is the tail). 1c A remains one WAL `fdatasync` per Ok.

FLOOR not enabled.
