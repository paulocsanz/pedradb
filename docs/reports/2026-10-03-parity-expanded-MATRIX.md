# Parity expanded campaign — `99b8174`

Config base: `ROCKS_PARITY_SYNC=0` (Rocks async = official peer) ·
`ROCKS_YCSB_OPS=100000` · `ROCKS_YCSB_PAYLOAD=100` · `ROCKS_DEPS_BATCH=32` ·
`ROCKS_PARITY_BIG=0` · `ROCKS_PARITY_MC_FRESH=1` · `TMPDIR=/tmp` · 3 runs · mediana.

## Escalas (RECORDS / ENTRIES)

| label | N |
|---|---:|
| 100k | 100_000 (oficial ycsb_b_mc4) |
| 1M | 1_000_000 |
| 10M | 10_000_000 |
| 25M | 25_000_000 (oficial overwrite_mc4) |
| 100M | 100_000_000 (oficial qs_neg / prefix open) |
| 250M | 250_000_000 |
| 500M | 500_000_000 |

## A. rocks-parity-bench — Pedra (`compat`) vs Rocks (`real`)

| Célula | Suite | CLIENTS | Escalas | Runs | Engines | Células |
|---|---|---:|---|---:|---|---:|
| deps_cache_overwrite_mc4 | ycsb,deps | 4 | 7 | 3 | compat+rocks | 42 |
| deps_apply_batch_mc4 | ycsb,deps | 4 | 7 | 3 | compat+rocks | 42 |
| ycsb_b_mc4 | ycsb,deps | 4 | 7 | 3 | compat+rocks | 42 |
| ycsb_f_mc4 | ycsb,deps | 4 | 7 | 3 | compat+rocks | 42 |
| qs_neg_lookup | qs | 1 | 7 | 3 | compat+rocks | 42 |

**Subtotal A:** 210 engine-runs → 105 compare cells (ratio por scale×run×shape).

Ordem: por escala ascendente → r1..r3 → shape group (mc4 batch, depois qs) → compat then rocks → compare.

## B. fjall same-binary (absolute QPS, never Rocks ratio)

| Célula | Suite | Escalas / note | Runs | Engines | Células |
|---|---|---|---:|---|---:|
| fjall_seq_64k | ladder | fixed 64k (harness) | 3 | compat+fjall | 6 |
| fjall_seq_1m | ladder | fixed 1M | 3 | compat+fjall | 6 |
| fjall_rand_rw_1m | ladder | fixed 1M (README) | 3 | compat+fjall | 6 |
| fjall_scan_1k | ladder | fixed scan ws | 3 | compat+fjall | 6 |
| ycsb_a_mc4 vs fjall | ycsb | 7 RECORDS scales | 3 | compat+fjall | 42 |

Fjall ladder sizes are **hardcoded** in harness (cannot scale to 25M without code change). Expanded via `ycsb_a_mc4` × RECORDS ladder.

## C. prefix scan bounded cache (open README cell)

Harness: `snapshot_backends` · `SLIPSTREAM_BENCH_CACHE_BYTES=4294967296` (4 GiB) ·
VALUE=200 · SEQUENTIAL=1 · backends pedradb,rocksdb (fjall also) ·
CELLS unset (full; prefix_scan is the named open cell).

| Escala | Runs | Backends | Células |
|---|---:|---|---:|
| 1M,10M,25M,100M,250M,500M | 3 | 3 | 54 |

## D. scale-parity-bench (fjall column)

**Blocked:** `--bin pedra -- scale` referenced in docs/scripts **does not exist** in this tree (Cargo.toml has no `pedra` bin). Substitute:

| Substitute | Escalas | CACHE | Runs | Backends | Células |
|---|---|---|---:|---|---:|
| snapshot_backends (protocol-ish) | 1M→500M | 256 MiB | 3 | fjall,rocks,pedra | 54 |

## Totals (runnable)

| Block | Cells (approx) |
|---|---:|
| A parity mc4+qs | 210 eng / 105 ratios |
| B fjall | 66 eng |
| C prefix 4GiB | 54 |
| D scale 256MiB | 54 |
| **Grand** | **~384 eng-runs** |

Risks: 250M/500M seed on 47 GiB host may OOM or be very slow; fjall 500M settle SKIP disk; SKIP_OOM recorded, no silent clamp.


## Runtime notes

- `deps_apply_batch` (1c) is included in `ROCKS_PARITY_ONLY` solely so the
  harness runs the MVCC deps seed required by `deps_apply_batch_mc4`
  (`need_seed` checks the 1c name only).
- Campaign running in tmux `parity-expanded-3x`; logs `/tmp/parity-scale-logs/`.
- Engine SHA: tree at `99b8174` (bench report commit on top of `b7fcde6` engine).
