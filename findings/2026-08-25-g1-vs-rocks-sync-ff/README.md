# G1 vs Rocks `sync=true` + `F_FULLFSYNC` — Darwin atual (este Mac)

**2026-08-25.** Same-class barrier: Pedra `PEDRA_PARITY_G1=1` (`set_sync(true)`,
`wal_full_fsync` → `F_FULLFSYNC`) vs Rocks `ROCKS_PARITY_SYNC=1`
`ROCKS_PARITY_FULL_SYNC=1` (WAL `sync_all` / `F_FULLFSYNC` — rust-rocksdb
sem `HAVE_FULLFSYNC` não faria isto sozinho).

Protocolo irmão do floor1x-g1: records=1024 ops=200 payload=100 uniform,
`ROCKS_PARITY_BIG=0`. 1 run. **Caixa suja** (load 15–23 / 16 users, M2 Max).
Não é bateria oficial 3/3 quieta. Binário: worktree `rfc-0054-gaps` @
`3e3fdde` + `PEDRA_PARITY_G1` (o tree ainda não tinha o selector; sem ele
a primeira tentativa mediu async vs FF e foi arquivada em
`../2026-08-25-async-vs-rocks-ff-NOT-G1/` — **não citar**).

JSON: compat `"sync": true`, durability `F_FULLFSYNC on Darwin`; rocks
`"sync": true`, durability `sync-on-commit + F_FULLFSYNC`.

## Ratios (qps Pedra G1 / Rocks FF)

| shape | Pedra qps | p50 P/R ms | Rocks qps | ratio |
|---|---:|---:|---:|---:|
| ycsb_c | 3 139 323 | 0.0003 / 0.0005 | 1 859 030 | **1.69** |
| ycsb_c_unif | 3 151 691 | 0.0003 / 0.0004 | 1 992 528 | **1.58** |
| deps_mvcc_latest | 311 952 | 0.0023 / 0.0035 | 205 400 | **1.52** |
| ycsb_b_unif | 4 115 | 0.0005 / 0.0013 | 3 299 | **1.25** |
| deps_lock_prewrite | 121 | 6.98 / 8.08 | 107 | **1.13** |
| deps_raftlog | 39.7 | 27.6 / 29.3 | 35.2 | **1.13** |
| ycsb_f | 218 | 0.004 / 0.008 | 292 | 0.75 |
| ycsb_b | 1 936 | 0.0005 / 0.0009 | 3 106 | 0.62 |
| ycsb_a | 198 | **4.85 / 4.73** | 324 | 0.61 |
| deps_cache_overwrite | 50 | 25.6 / 8.2 | 88 | 0.57 |
| deps_scan | 165 005 | 0.0033 / 0.0032 | 298 767 | 0.55 |
| ycsb_e | 2 000 | 0.002 / 0.004 | 4 456 | 0.45 |
| ycsb_d | 416 | 0.0005 / 4.87 | 2 452 | 0.17 |
| deps_apply_batch | 6.3 | 59 / 15 | 37 | 0.17 |

## Como ler (não é 0.001×)

- **ycsb_a p50 empatou** (4.85 vs 4.73 ms). Os dois pagam o mesmo
  `F_FULLFSYNC`. O 0.61× é cauda (p99 28 vs 9 ms) numa caixa a load 20 —
  não “somos 40% mais lentos no syscall”.
- **apply 0.17× e ycsb_d 0.17× são contaminação** (apply max **2.9 s**,
  d max **379 ms**). Não usar como cartaz. Remesura quieta é RFC-0062 P1.1.
- Leituras puras **ganham** (C 1.69, mvcc 1.52). raftlog 1.13 com p50
  empatado (~28 ms) — um commit, uma barreira, 16 puts.
- Contra a coluna C (G1 vs Rocks **async**, floor1x-g1): ycsb_a era
  **0.001×** porque o peer não sincava. Aqui o peer sinca. O 1000× some.

Não é vitória oficial (AGENTS.md: peer `sync=true` não é o Rocks default).
É a resposta a “G1 vs Rocks sync, Darwin atual”.
