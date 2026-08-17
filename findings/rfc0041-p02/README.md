# RFC-0041 P0.2 — remesura oficial vs Rocks **default**

2026-08-17. Peer único: `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
Pedra sempre `fdatasync` antes do Ok. Harness:
`ROCKS_PARITY_CLIENTS=4 ROCKS_PARITY_SYNC=0 ROCKS_PARITY_SUITE=ycsb,deps scripts/tikv_ycsb_parity_v0.sh`
em `findings/rfc0041-p02/run{1,2,3}/`. 4096 records / 2000 ops / 1 KB / zipfian.
`rocks-parity-compare` recusou `sync=true` (não usado).

Isolado na mesma caixa (`scripts/measure_fdatasync.sh`):
`fdatasync n=200 p50=25.7 µs p95=88.3 µs` → `1/p50 ≈ 38.9 k` qps por um fd
síncrono de um cliente.

## Mediana de 3 runs (`compat_qps / rocks_default_qps`)

| shape | ratio | Pedra qps | Rocks qps | Pedra p50 | Rocks p50 | ≥ 2.0 |
|---|---:|---:|---:|---:|---:|:---:|
| ycsb_a | 0.056 | 22 340 | 398 347 | 25.8 µs | 2.3 µs | no |
| ycsb_b | 0.203 | 175 375 | 1 060 890 | 0.7 µs | 0.7 µs | no |
| ycsb_c | 0.777 | 1 091 455 | 1 286 864 | 0.3 µs | 0.7 µs | no |
| ycsb_d | 0.161 | 152 267 | 1 010 633 | 0.7 µs | 0.8 µs | no |
| ycsb_e | 0.909 | 111 891 | 122 974 | 3.8 µs | 7.0 µs | no |
| ycsb_f | 0.073 | 22 214 | 309 135 | 29.2 µs | 3.1 µs | no |
| deps_apply_batch | 0.389 | 2 537 | 6 483 | 165 µs | 111 µs | no |
| deps_mvcc_latest | 0.786 | 217 606 | 276 868 | 0.9 µs | 3.2 µs | no |
| deps_scan | 0.500 | 136 110 | 254 724 | 0.3 µs | 3.4 µs | no |
| deps_raftlog | 0.466 | 7 084 | 33 132 | 49.5 µs | 15.5 µs | no |
| deps_cache_overwrite | 0.196 | 23 490 | 128 456 | 32.4 µs | 2.8 µs | no |
| ycsb_a_mc4 | 0.205 | 27 726 | 95 532 | 32.0 µs | 3.5 µs | no |
| ycsb_f_mc4 | 0.244 | 33 549 | 123 133 | 59.4 µs | 4.1 µs | no |
| deps_cache_overwrite_mc4 | 0.517 | 24 957 | 62 352 | 104 µs | 17.9 µs | no |
| **deps_apply_batch_mc4** | **0.888** | 3 120 | 3 658 | 852 µs | 729 µs | no |
| deps_raftlog_mc4 | 0.529 | 9 499 | 18 705 | 220 µs | 134 µs | no |

**0 / 16 ≥ 2.0.** P0.3 não liga `ROCKS_PARITY_RATIO_FLOOR=2.0` (conjunto vazio).

Mais perto: `ycsb_e` 0.909 (scan/read), `apply_mc4` 0.888 (escrita). Run-a-run
`apply_mc4`: 0.809 / 1.218 / 0.888. Run2 infla razões porque o Rocks caiu
(apply 3.1 k vs ~6.5 k); a mediana é o número.

## O que o número fecha

- **1 cliente write + 1 fd/Ok não chega a 2× este Rocks.** YCSB A Rocks
  ~398 k; 2× = 797 k. Teto físico de um fd síncrono nesta caixa ≈ 39 k.
  Pedra A mediana 22 k (abaixo do teto — ainda há CPU). Mesmo no teto
  seria 0.10×. P1.2/P1.3 1c **ficam `todo`**. Alvo e peer **não** mudam.
- **apply_mc4 não é o fd.** p50 852 µs, fd isolado 26 µs (~3 %). 3.1 k qps
  vs 1/fd = 39 k. Para 2× Rocks (~7.3 k) falta ~2.25× de CPU/lock/cauda,
  não mais group-wait. Catch-up 200 µs já rejeitado (0040 P1.2).
- **raftlog_mc4** 0.529 (p50 220 vs 134 µs). Precisa ~3.8×. Group ~1.6
  ainda deixa ~1 fd por write; 4 clientes não bastam.
- **Leituras já ganham no p50** (C, E, MVCC, scan) e perdem no qps por
  cauda L0/p95. Isso é P2, não P1.

## Raw

- `fdatasync.txt`
- `run{1,2,3}/{compat,rocks}/rocks_parity_bench.json`
- `run{1,2,3}/compare/compare_report.json`
