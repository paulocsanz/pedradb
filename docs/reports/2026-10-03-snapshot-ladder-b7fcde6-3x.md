# Progress — 1M / 10M / 25M / 100M / 250M / 500M × 3 — `b7fcde6`
**Config default:** harness completo · `VALUE=200` · `SEQUENTIAL=1` · `CACHE=unset` (~1 GiB) · `PEDRA_STAGE_MAX_BYTES=unset` · `TMPDIR=/tmp`
Só células desta campanha (master `bench_ladder_3x_*.log`). Tag ≠ default ou FAIL/SKIP anotado.
## Scoreboard

| Scale | r1 fjall | r1 rocks | r1 pedra | r2 fjall | r2 rocks | r2 pedra | r3 fjall | r3 rocks | r3 pedra |
|---|---|---|---|---|---|---|---|---|---|
| 1M | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 10M | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 25M | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 100M | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 250M | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 500M | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

Progresso: **54/54** células com exit.

### 1M — mediana (só `code=0`)

| Metric | fjall | RocksDB | PedraDB |
|---|---|---|---|
| Hydrate | 0.8s / 1.18 M/s → 0.32 GiB | 0.6s / 1.62 M/s → 0.22 GiB | 0.3s / 3.27 M/s → 0.20 GiB |
| Settle | 0.0s → 0.33 GiB | 0.8s → 0.21 GiB | 0.1s → 0.24 GiB |
| Cold hit p50 | 2.90 µs | 11.70 µs | 12.90 µs |
| Cold miss p50 | 434.0 ns | 579.0 ns | 140.0 ns |
| probe_miss (crit) | 276.3 ns | 560.7 ns | 110.7 ns |
| get_hit | 1.78 µs | 2.14 µs | 2.23 µs |
| prefix_scan | 203.93 µs | 156.16 µs | 125.87 µs |
| lookup get_loop | 167.16 µs | 209.83 µs | 213.80 µs |
| lookup multi_get | 178.46 µs | 184.06 µs | 221.12 µs |


### 10M — mediana (só `code=0`)

| Metric | fjall | RocksDB | PedraDB |
|---|---|---|---|
| Hydrate | 10.0s / 1.00 M/s → 2.41 GiB | 6.0s / 1.67 M/s → 2.49 GiB | 3.0s / 3.38 M/s → 2.35 GiB |
| Settle | 6.7s → 4.36 GiB | 3.1s → 2.10 GiB | 0.2s → 2.40 GiB |
| Cold hit p50 | 9.60 µs | 13.30 µs | 14.90 µs |
| Cold miss p50 | 380.0 ns | 304.0 ns | 140.0 ns |
| probe_miss (crit) | 336.2 ns | 281.0 ns | 110.6 ns |
| get_hit | 7.52 µs | 6.52 µs | 8.70 µs |
| prefix_scan | 203.50 µs | 153.58 µs | 127.87 µs |
| lookup get_loop | 745.23 µs | 664.37 µs | 856.21 µs |
| lookup multi_get | 784.84 µs | 741.07 µs | 874.71 µs |


### 25M — mediana (só `code=0`)

| Metric | fjall | RocksDB | PedraDB |
|---|---|---|---|
| Hydrate | 24.6s / 1.02 M/s → 5.46 GiB | 14.7s / 1.70 M/s → 6.78 GiB | 7.4s / 3.39 M/s → 5.94 GiB |
| Settle | 18.6s → 9.92 GiB | 4.1s → 5.24 GiB | 0.2s → 5.99 GiB |
| Cold hit p50 | 18.10 µs | 10.30 µs | 15.60 µs |
| Cold miss p50 | 390.0 ns | 310.0 ns | 140.0 ns |
| probe_miss (crit) | 348.5 ns | 286.1 ns | 111.2 ns |
| get_hit | 9.36 µs | 8.96 µs | 10.85 µs |
| prefix_scan | 204.38 µs | 153.70 µs | 133.34 µs |
| lookup get_loop | 926.24 µs | 884.71 µs | 1.05 ms |
| lookup multi_get | 983.56 µs | 983.63 µs | 1.08 ms |


### 100M — mediana (só `code=0`)

| Metric | fjall | RocksDB | PedraDB |
|---|---|---|---|
| Hydrate | 101.6s / 0.98 M/s → 21.41 GiB | 60.5s / 1.65 M/s → 24.28 GiB | 29.8s / 3.35 M/s → 23.88 GiB |
| Settle | 76.6s → 41.10 GiB | 5.9s → 20.98 GiB | 0.2s → 23.94 GiB |
| Cold hit p50 | 50.80 µs | 40.70 µs | 48.20 µs |
| Cold miss p50 | 443.0 ns | 333.0 ns | 140.0 ns |
| probe_miss (crit) | 396.3 ns | 309.8 ns | 110.4 ns |
| get_hit | 27.18 µs | 34.27 µs | 31.33 µs |
| prefix_scan | 204.37 µs | 166.65 µs | 136.42 µs |
| lookup get_loop | 2.55 ms | 3.25 ms | 3.25 ms |
| lookup multi_get | 2.78 ms | 3.27 ms | 3.45 ms |


### 250M — mediana (só `code=0`)

| Metric | fjall | RocksDB | PedraDB |
|---|---|---|---|
| Hydrate | 259.7s / 0.96 M/s → 52.79 GiB | 147.0s / 1.70 M/s → 55.16 GiB | 90.8s / 2.75 M/s → 59.79 GiB |
| Settle | 200.8s → 102.00 GiB | 5.7s → 52.45 GiB | 0.6s → 59.85 GiB |
| Cold hit p50 | 77.90 µs | 44.40 µs | 54.90 µs |
| Cold miss p50 | 419.0 ns | 373.0 ns | 141.0 ns |
| probe_miss (crit) | 380.4 ns | 347.8 ns | 110.2 ns |
| get_hit | 43.50 µs | 38.33 µs | 41.94 µs |
| prefix_scan | 204.82 µs | 169.98 µs | 177.39 µs |
| lookup get_loop | 3.72 ms | 3.81 ms | 4.24 ms |
| lookup multi_get | 3.62 ms | 4.14 ms | 4.39 ms |


### 500M — mediana (só `code=0`)

| Metric | fjall | RocksDB | PedraDB |
|---|---|---|---|
| Hydrate | 547.0s / 0.91 M/s → 105.31 GiB | 296.3s / 1.69 M/s → 108.57 GiB | 212.9s / 2.35 M/s → 119.65 GiB |
| Settle | — | 10.2s → 104.90 GiB | 1.2s → 119.70 GiB |
| Cold hit p50 | 46.60 µs | 65.10 µs | 58.40 µs |
| Cold miss p50 | 436.0 ns | 372.0 ns | 141.0 ns |
| probe_miss (crit) | 394.8 ns | 349.6 ns | 113.6 ns |
| get_hit | 47.69 µs | 238.91 µs | 68.50 µs |
| prefix_scan | 204.28 µs | 170.48 µs | 171.17 µs |
| lookup get_loop | 5.92 ms | 21.04 ms | 7.04 ms |
| lookup multi_get | 6.24 ms | 22.13 ms | 8.14 ms |


