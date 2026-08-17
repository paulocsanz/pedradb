# RFC-0041 — park + pairwise fold, BTree clone off the Db lock

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`parkfold2/run{1,2,3}`). JSONs only. Peers `sync: false`.

Same worker as parkfold (park during writes, fold when `active ≤ 1`
and not recently multi, materialize only after 200 ms idle). Parked
tables are `Arc<MemTable>` so fold snapshots with `Arc::clone` and
deep-clones **off** the Db lock (parkfold MVCC/apply tails were the
clone under the read lock). G1: parked keys stay on WAL; rotate waits.

## Result

**2/16 ≥ 2.0** (`deps_scan` **2.209**, `ycsb_e` **2.064**). FLOOR not
enabled. All peers `sync: false`. L0=0 / `scan_sst_probed=0` at
MVCC and scan on every run.

| shape | scansst | wake2 | parkfold2 med | Pedra qps |
|---|---:|---:|---:|---:|
| deps_scan | 1.152 | 0.205 | **2.209** | 417 900 |
| ycsb_e | 0.913 | 1.002 | **2.064** | 182 265 |
| deps_mvcc_latest | **2.580** | 1.098 | 1.964 | 289 946 |
| ycsb_c | 1.517 | 1.750 | 1.607 | 1 769 324 |
| apply_mc4 | 0.956 | 1.484 | **1.687** | 4 271 |
| apply 1c | — | 0.531 | 0.628 | 3 081 |
| raftlog_mc4 | — | 0.694 | 0.615 | 8 941 |
| ycsb_a | 0.099 | 0.084 | 0.123 | 34 558 |

1c A/F still one WAL `fdatasync` per Ok (p50 ~26–30 µs). 2× Rocks A
(~300 k) remains above `1/t_fd`. Target and peer unchanged.
