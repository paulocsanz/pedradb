# Darwin coluna B — isolated retry after harness hang

**When:** 2026-08-27T04:08:04Z–04:09:47Z (~100 s, 3 rounds)  
**Box:** M2 Max, load 9–11 / 18–19 users — **not** quiet 3/3.  
**Peer:** Pedra `PEDRA_PARITY_G1=1` vs Rocks `sync=true` + `ROCKS_PARITY_FULL_SYNC=1`.  
**Shapes:** `ROCKS_PARITY_ONLY=ycsb_a,deps_raftlog` ops=2000 zipfian.

## Harness bugs this run found

1. `ROCKS_PARITY_ONLY` filtered **deps/kvrocks** but **not YCSB**. Isolated intent still ran `ycsb_c_big` (2^20 seed).
2. `ROCKS_PARITY_FULL_SYNC=1` called `File::sync_all` on **every** `put`, including untimed seeds that flip `set_write_sync(false)`. 1M Darwin `F_FULLFSYNC` ≈ 1 h hang. Killed the first attempt.

Fixes in `rocksdb-parity-bench`: `shape_wanted` on YCSB; `full_sync_wal` follows `cur_sync`.

## Numbers (dirty)

| shape | min qps | mediana | rounds P/R qps | p50 P/R ms |
|---|---:|---:|---|---|
| ycsb_a | **0.978** | 1.000 | 1.338 / **0.978** / 1.000 | 3.73/3.42, 3.55/3.41, 3.65/3.15 |
| deps_raftlog | **0.984** | 1.041 | 1.204 / **0.984** / 1.041 | 4.01/4.12, 4.01/4.01, 4.00/4.03 |

p50 **empatado** no syscall (~3.5 ms A, ~4.0 ms raftlog). min < 1.0 é cauda (r1 rocks p99 15/25 ms), não o p50. Não é gate Darwin 3/3.

JSON: `r{1,2,3}/{pedra,rocks}/rocks_parity_bench.json`.
