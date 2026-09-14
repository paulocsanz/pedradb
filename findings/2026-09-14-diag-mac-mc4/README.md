# DIAG Darwin — overwrite_mc4 + ycsb_f_mc4 vs Rocks default

**When:** 2026-09-14. **Host:** this Mac, load 5-min ~16. **Not** cartaz.
**Peer:** `ROCKS_PARITY_SYNC=0`, JSON `sync: false`. Same-class async.
**Harness:** `SUITE=ycsb` `CLIENTS=4` `ONLY=<shape>` `RECORDS=100000`
`OPS=100000` zipf payload=100. Interleaved rocks→compat ×3.

`SUITE=deps` does **not** run overwrite_mc4 (that lives in `run_clients`).

## overwrite_mc4

| r | pedra qps | rocks qps | ratio | pedra p50/p99/max | rocks p50/p99/max | Rocks ≳260k? |
|---|---:|---:|---:|---|---|---|
| 1 | 83 932 | 162 040 | 0.518 | 10.1 / 579 / 47 639 | 13.8 / 82 / 57 117 | **no** (collapsed) |
| 2 | 80 424 | 229 475 | 0.351 | 10.2 / 600 / 27 909 | 13.9 / 84 / 10 201 | borderline |
| 3 | 86 723 | 282 285 | **0.307** | 9.5 / 625 / 27 917 | 13.3 / 27 / 8 535 | yes |

Valid-ish round = r3. Pedra **ganha p50** (9.5 vs 13.3 µs) e **perde QPS
3×** porque p99 625 vs 27 µs — cauda, não o mediano. Mapa Darwin 100k
era 0.716× (164 k vs 229 k); hoje 0.31–0.35 sob load 16. Não substitui
Linux 0.557× @25M.

## ycsb_f_mc4

| r | pedra qps | rocks qps | ratio | pedra p50/p99/max | rocks p50/p99/max |
|---|---:|---:|---:|---|---|
| 1 | 125 095 | 457 733 | 0.273 | 9.8 / 299 / 31 015 | 4.2 / 32 / 11 280 |
| 2 | 64 493 | 441 318 | **0.146** | 8.4 / 356 / **186 351** | 4.5 / 48 / 9 025 |
| 3 | 130 394 | 463 281 | 0.282 | 9.5 / 322 / 29 168 | 4.4 / 23 / 74 506 |

Rocks 441–463 k saudável. Pedra p50 ~2× e rabo (r2 max 186 ms). min
**0.146**. Linux cartaz continua mediana 1.47 / run2 0.766.

## 1B hat (kernel, no runtime)

`pedra scale-model --keys 1000000000 --ram 68719476736`:
S=245 GiB L=4 P_best=5 P_worst=8 n_files=913 mode=bounded-cache
T happy=60.6 µs worst=162 µs. Same P on this 96 GiB Mac (store still
does not fit).
