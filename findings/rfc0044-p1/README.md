# RFC-0044 P1 — kvrocks async/async (dirty)

**Not official. Not the product. Not “we beat Rocks.”**

Column: Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `WriteOptions.sync=false`.
Mix: 4096 / 2000 / 1 KB / zipfian / `ROCKS_DEPS_BATCH=32`.
G1 default is unchanged.

## `kvrocks/` — first remesure (shared-Bytes pipeline only)

Load: dirty. `compat.sync=false` `rocks.sync=false`.

| shape | Pedra | Rocks | ratio | vs x5b | ≥5? |
|---|---:|---:|---:|---:|:---:|
| `kvrocks_scan` | 804k | 20.7k | **38.85** | 44 | yes |
| `kvrocks_set_mc50` | 237k | 55.6k | 4.27 | 5.07 | no (was yes) |
| `kvrocks_set` | 588k | 205k | 2.88 | 2.14 | no |
| `kvrocks_get` | 1.87M | 1.10M | 1.70 | 1.43 | no |
| `kvrocks_pipelined_set` | 12.3k | 11.3k | 1.09 | 1.56 | no |
| `kvrocks_blob_set` | 42.2k | 48.6k | 0.87 | 1.96 | **lose** |

Pipeline p50 Pedra 37 µs vs Rocks 51 µs (already faster on median); wall
mean ~81 µs. Interned `Bytes` in `put_batch_same` did **not** close 5×.

Load at remesure start: **29 / 39 / 46 on 12 CPUs** (`uptime` 17:58). Dirty.

## `kvrocks2/` — WAL v2 reuse + intern + insert_many + get_probe

Code (RFC-0044 P1.1/P1.3, this slice):

- WAL record v2: consecutive interned values stored once (`kind | 0x80`)
- `BatchOp::put` interns identical payloads (SET / blob)
- `MemTable::insert_many` (one `tail_ord` invalidate per batch)
- `put_batch_same` moves default-CF keys (`Bytes::from`)
- dirty-points gen-bump at `>= 32` (pipeline is 32)
- `get_probe` / `DB::contains` (no 1 KB `to_vec` on GET canary)

Load: 35–49 / 12 CPUs. `compat.sync=false` `rocks.sync=false`.

Pedra **CPU** (vs `kvrocks/` this folder): pipeline 12.3k → **50.3k** (p50 37 µs → 13 µs);
blob 42k → **100k**. WAL v2 + intern worked on Pedra.

Rocks on this run is **sick** (SET 24k vs 205k on `kvrocks/`; pipeline 5.7k vs 11.3k).
Do **not** sell 21× / 8.85× as the 5× close.

| shape | Pedra | Rocks this run | ratio | vs healthier Rocks `kvrocks/` | ≥5 vs healthy? |
|---|---:|---:|---:|---:|:---:|
| `kvrocks_scan` | 683k | 13.5k | **50.6** | 33 | yes |
| `kvrocks_set` | 515k | 24.3k | 21.2† | 2.52 | no |
| `kvrocks_pipelined_set` | **50.3k** | 5.7k | 8.85† | **4.45** | no (close) |
| `kvrocks_set_mc50` | 224k | 43.8k | **5.11** | 4.03 | p0 was 5.07 |
| `kvrocks_blob_set` | 100k | 26.6k | 3.77† | 2.06 | no |
| `kvrocks_get` | 1.33M | 1.03M | 1.30 | 1.21 | no |

† Rocks qps collapsed under load; Pedra p50 is the number that counts until a quiet 3× (P2.1).

## `kvrocks3/` — WAL 1-op coalesce + keys fora da janela

Load ~50 / 12 CPUs. `compat.sync=false` `rocks.sync=false`.

Pedra: SET p50 **0.9 µs** (981 k), pipeline p50 **5 µs** (134 k), blob p50 **0.8 µs** (1.0 M).
Rocks nesta run está **sano** no SET/pipeline (248 k / 12.8 k), ao contrário de `kvrocks2/`.

| shape | Pedra | Rocks | this | vs Rocks `kvrocks/` | ≥5? |
|---|---:|---:|---:|---:|:---:|
| `kvrocks_pipelined_set` | **134 k** | 12.8 k | **10.5** | **11.8** | **sim** |
| `kvrocks_blob_set` | **1.00 M** | 48.2 k | **20.8** | **20.6** | **sim** |
| `kvrocks_set` | 981 k | 248 k | 3.95 | **4.79** | quase (quiet) |
| `kvrocks_set_mc50` | 267 k | 54.9 k | 4.86 | 4.80 | p0 5.07 |
| `kvrocks_get` | 3.66 M | 1.58 M | 2.32 | 3.33 | não (p50 <50 ns; wall=cauda) |
| `kvrocks_scan` | 46 k | 24.9 k | 1.85 | 2.23 | **não nesta run** (max 35 ms); x5b/kvrocks2 era 33–44× |

Pipeline e blob fecham o piso contra o peer saudável. SET 1c falta ~4% (981 k vs 1.02 M).
GET 5× é cauda, não p50. SCAN 35 ms = uma parada na caixa suja — não reescrever o 33× anterior.

## `kvrocks4/` + `ycsb/` — get_probe + ytab no YCSB

Load 14–25 / 12 CPUs. Ainda dirty.

Kvrocks vs Rocks `kvrocks/` (peer 205 k SET / 11.3 k pipeline):

| shape | Pedra | this Rocks | this | vsH | ≥5 vsH? |
|---|---:|---:|---:|---:|:---:|
| set | **1.02 M** | 139 k | 7.37† | **5.00** | **sim** |
| pipeline | 112 k | 15.2 k | **7.39** | **9.92** | **sim** |
| blob | 808 k | 54.3 k | **14.9** | **16.6** | **sim** |
| scan | 321 k | 23.0 k | **13.9** | **15.5** | **sim** |
| mc50 | 264 k | 77.1 k | 3.43 | 4.76 | p0 5.07 |
| get | 3.78 M | 1.65 M | 2.29 | 3.44 | não |

† Rocks SET 139 k vs 205 k saudável — o 7.37 não é o close; o 5.00 vsH é.

YCSB same-run (os dois com ytab/`get_probe`):

| shape | Pedra | Rocks | ratio | vs x5b Rocks |
|---|---:|---:|---:|---:|
| A | 683 k | 372 k | 1.84 | 2.32 |
| B | 1.19 M | 1.11 M | 1.07 | 1.47 |
| C | 2.66 M | 1.58 M | 1.69 | 3.81 |
| D | 1.79 M | 1.23 M | 1.46 | 3.55 |
| E | 435 k | 111 k | 3.91 | 5.92 |
| F | 549 k | 297 k | 1.85 | 2.22 |

Rocks também acelerou com keys fora da janela; same-run E caiu de 5.61 (x5b) para 3.91. P2.2 não fecha nesta caixa.

## `kvrocks4/` + `ycsb/` — get_probe + ytab no YCSB

Load 14–25 / 12 CPUs. Ainda dirty.

Kvrocks vs Rocks `kvrocks/` (peer 205 k SET / 11.3 k pipeline):

| shape | Pedra | this Rocks | this | vsH | ≥5 vsH? |
|---|---:|---:|---:|---:|:---:|
| set | **1.02 M** | 139 k | 7.37† | **5.00** | **sim** |
| pipeline | 112 k | 15.2 k | **7.39** | **9.92** | **sim** |
| blob | 808 k | 54.3 k | **14.9** | **16.6** | **sim** |
| scan | 321 k | 23.0 k | **13.9** | **15.5** | **sim** |
| mc50 | 264 k | 77.1 k | 3.43 | 4.76 | p0 5.07 |
| get | 3.78 M | 1.65 M | 2.29 | 3.44 | não |

† Rocks SET 139 k vs 205 k saudável — o 7.37 não é o close; o 5.00 vsH é.

YCSB same-run (os dois com ytab/`get_probe`):

| shape | Pedra | Rocks | ratio | vs x5b Rocks |
|---|---:|---:|---:|---:|
| A | 683 k | 372 k | 1.84 | 2.32 |
| B | 1.19 M | 1.11 M | 1.07 | 1.47 |
| C | 2.66 M | 1.58 M | 1.69 | 3.81 |
| D | 1.79 M | 1.23 M | 1.46 | 3.55 |
| E | 435 k | 111 k | 3.91 | 5.92 |
| F | 549 k | 297 k | 1.85 | 2.22 |

Rocks também acelerou com keys fora da janela; same-run E caiu de 5.61 (x5b) para 3.91. P2.2 não fecha nesta caixa.
