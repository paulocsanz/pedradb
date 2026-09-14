# DIAG Linux Railway — overwrite_mc4 + ycsb_f_mc4 vs Rocks default

**When:** 2026-09-14T18:06–18:26Z. **Not** cartaz (322 GiB RAM, load 15–24).
**Peer:** `ROCKS_PARITY_SYNC=0`, JSON `sync: false`.
**Host:** AMD EPYC 9655P, nproc=48, Mem 322 GiB, `/data` 46 GiB.
**Deploy:** `572da818` SUCCESS. Project `pedra-linux-wave`.

Protocol: round with Rocks overwrite_mc4 < 260 kQPS is invalid.

## Canary overwrite_mc4 100k

| r | pedra | rocks | ratio | ≥260k |
|---|---:|---:|---:|---|
| 1 | 143 725 | 344 131 | 0.418 | yes |
| 2 | 132 080 | 245 033 | 0.539 | **no** |
| 3 | 113 135 | 274 511 | 0.412 | yes |

Valid min **0.412**. Same band as Darwin 100k (0.31).

## overwrite_mc4 25M (hot on this box — not 4 GiB leftover)

| r | pedra | rocks | ratio | ≥260k |
|---|---:|---:|---:|---|
| 1 | 96 877 | 251 671 | 0.385 | borderline |
| 2 | 100 142 | 282 922 | **0.354** | yes |
| 3 | 100 740 | 270 957 | 0.372 | yes |

`ow25_RESULT min=0.354 med=0.372`. Dataset ~6 GiB on 322 GiB RAM = hot.
Leftover DONTNEED does not explain this (Fire-119 is bounded-cache).
Same owner as 100k: group/tail, `avg_grp=1`.

## ycsb_f_mc4 100k

| r | pedra | rocks | ratio |
|---|---:|---:|---:|
| 1 | 201 175 | 788 927 | 0.255 |
| 2 | 172 452 | 681 120 | 0.253 |
| 3 | 192 712 | 806 793 | **0.239** |

Rocks 681–807 k healthy. min **0.239**. Darwin DIAG min 0.146.

## Not paid

Linux cartaz `overwrite_mc4` 0.557 @ 4 GiB and `ycsb_f_mc4` run2 0.766
stay. This run is DIAG Linux noisy / fat RAM.
