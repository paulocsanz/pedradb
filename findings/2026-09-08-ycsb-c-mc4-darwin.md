# ycsb_c_mc4 Darwin DIAG — RFC-0184 P2.39

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`)
**Host:** Darwin DIAG — not Linux cartaz
**Meter:** `/tmp/pedra-vs-rocks-c75`

## Number

```
number: ratio=0.909 pedra_qps=4708218 rocks_qps=5179518 shape=ycsb_c_mc4 (DIAG)
```

| | Pedra compat | Rocks default |
|---|---|---|
| QPS | 4 708 218 | 5 179 519 |
| n | 200 000 | 200 000 |
| p50_ms | 0.0005 | 0.0005 |
| p99_ms | 0.0023 | 0.002 |
| max_ms | 0.239 | 0.0753 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |

Named loss kept. Rocks 5.18 M QPS is quiet for 100% get zipf 100 k (not collapsed).

## Shape

YCSB C 100% get, 4 clients, zipfian, records=100000 ops=50000. Official 16 only had 1c `ycsb_c`. Landed in `COMPARE_SHAPES` + `run_clients` + `BALANCE_SHAPES`. Test `rfc0184_ycsb_c_mc4_in_compare`.

p50 tied at 0.5 µs. 9% QPS hole is tail (Pedra max 239 µs vs Rocks 75 µs), not the zipf hit. Do not overfit a get_path cut on this 100 k Darwin cell.

## Rank leftover (unchanged)

- overwrite_mc4 Linux 0.557× unpaid; leftover+L0 Darwin 0.91×
- ycsb_f_mc4 Linux run2 0.766× unpaid
- ycsb_b_mc4 Darwin tiny 0.060× — do not cut on that size
