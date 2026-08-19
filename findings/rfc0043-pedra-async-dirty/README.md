# Pedra **async WAL** vs Rocks **async WAL** (same class)

**Not official. Not the product. Not “we beat Rocks”.**

Product (AGENTS.md / RFC-0041): Pedra **fdatasync before Ok** vs Rocks
default `WriteOptions.sync=false`. This folder drops G1 on purpose
(`PEDRA_PARITY_ASYNC=1`) so both engines skip WAL `fdatasync`. It
answers “if Pedra were Rocks-shaped async, how would the LSM compare?”

- Mix: 4096 / 2000 / 1 KB / zipfian / `ROCKS_DEPS_BATCH=32`
- `ROCKS_PARITY_CLIENTS=4` on the official 16
- Load ~49–73 / 12 CPUs — dirty, one run
- JSON `compat.sync=false` `rocks.sync=false`
- Durability label: `async-wal (PEDRA_PARITY_ASYNC=1; … NOT G1, not official)`

## Which suites already run Rocks async?

`write_sync_for_suite` is **false** unless `ROCKS_PARITY_SYNC=1`.

| suite | Rocks `WriteOptions.sync` | why |
|---|---|---|
| **ycsb, deps** (official 16) | **false** | Rocks default; official peer |
| qs | false | |
| kvrocks | false | Kvrocks `rocksdb.write_options.sync no` |
| nebula, streaming, solana, arango, venice, oxigraph | false | Rocks default |
| myrocks | **true** | `flush_log_at_trx_commit=1` |
| surreal | **true** | `SURREAL_DATASTORE_SYNC_DATA=every` |
| ceph | **true** | BlueStore omap is durable metadata |

This remesure covers every **async** suite. MyRocks/Surreal/Ceph were
not included (their host default is sync).

## Official 16 — Pedra async vs Rocks async

| shape | Pedra/Rocks async | note |
|---|---:|---|
| ycsb_e | **7.31** | scan |
| ycsb_c | **1.78** | point get |
| ycsb_d | **1.41** | |
| ycsb_f | **1.45** | RMW 1c |
| ycsb_a | **1.11** | 50/50 write mix 1c — G1 canário era ~0.23–0.57 |
| ycsb_b | 0.87 | |
| deps_lock_prewrite | **2.55** | |
| deps_apply_batch | **2.16** | |
| deps_apply_batch_mc4 | **2.06** | |
| deps_scan | **2.13** | |
| deps_raftlog | **1.90** | |
| deps_mvcc_latest | **1.60** | |
| ycsb_a_mc4 | 0.68 | |
| deps_raftlog_mc4 | 0.51 | grupo 1.36 nesta run |
| deps_cache_overwrite | 0.42 | |
| deps_cache_overwrite_mc4 | 0.31 | |
| ycsb_f_mc4 | 0.25 | |

## Kvrocks / QS / expanding (async vs async)

| shape | ratio |
|---|---:|
| kvrocks_get | 2.99 |
| kvrocks_scan | 6.44 |
| kvrocks_pipelined_set | 0.73 |
| kvrocks_set_mc50 | 0.61 |
| kvrocks_set 1c | **0.26** | even without fd: encode/lock vs Rocks put |
| kvrocks_blob_set | 0.23 |
| qs_batch_write | 2.37 |
| qs_neg_lookup | 1.26 |
| qs_hot_get | 0.19 |
| nebula_get_neighbors | 1.62 |
| nebula_insert_edge | 1.07 |
| kafka_changelog_flush | 10.8 |
| flink_window_state | 0.91 |
| solana_shred_append | 1.55 |
| solana_trailing_read | 7.34 |
| arango_doc_crud | 0.48 |
| arango_traversal | 1.33 |
| venice_fanout_get | 2.13 |
| rockstore_widecol_rw | 0.63 |
| oxigraph_spo_lookup | 1.97 |
| oxigraph_triple_put | 1.18 |

## How to read this

- **G1 is the whole canary hole.** ycsb_a 1c goes 0.23–0.57 (G1 vs async)
  → **1.11** when both skip fdatasync. The product still fsyncs.
- **1-op put still loses without fd** (`kvrocks_set` 0.26): WAL encode +
  write lock + mem apply vs Rocks’ cheaper put. Not a syscall class gap.
- **Reads / batches / scans** already beat Rocks in both columns.
- Seed 4096 puts: 0.0 s Pedra async (was seconds with G1) — the knob
  is doing what it says.

Env: `PEDRA_PARITY_ASYNC=1` on the **compat** binary only. Default
open stays `OpenOptions.sync=true`.

## wal-buf (after staging WAL until 32 KiB)

Same class, still not official. 1-op put no longer `write()`s every SET
(`write_pending_frame_if`): frame stays in userspace until a WAL block
or close/sync. G1 path still force-writes + fdatasync.

| shape | before buf | after buf |
|---|---:|---:|
| kvrocks_set 1c | 0.26 | **1.65** (212k / 129k) |
| kvrocks_set_mc50 | 0.61 | **1.25** |
| kvrocks_blob_set | 0.23 | **1.13** |
| ycsb_a | 1.11 | **4.10** |
| ycsb_b | 0.87 | **2.28** |
| ycsb_f | 1.43 | 1.43 |
| pipeline | 0.73 | 0.75 (already 1 write/batch) |
| kvrocks_get | 2.99 | 1.59 |
| ycsb_c | 1.78 | 0.32 this run (L0 after faster writes; dirty) |

Raw: `wal-buf/kvrocks/`, `wal-buf/ycsb/`.

## hotpath + rawcf (notify-off + default CF raw)

Compact worker used to `try_send` on **every put** (wakes a thread 200k/s).
Now it only polls (5 ms) + flush. Default CF is unprefixed unless deps/myrocks
need extra CFs.

`rawcf/` (async vs async, dirty):

| shape | ratio | qps P / R |
|---|---:|---|
| kvrocks_pipelined_set | **1.31** | 13.5k / 10.3k |
| kvrocks_set 1c | 1.62 | 249k / 153k |
| kvrocks_scan | **12.9** | |
| kvrocks_get | 1.58 | |
| blob_set | 1.34 | |
| set_mc50 | 0.67 | |
| ycsb_e | 4.69 | |
| ycsb_c | 3.19 | |
| ycsb_a | 1.42 | |

Pipeline **>1**. 5× on 1-op put **not** reached (need ~750k vs 150k;
CPU of encode+mem+lock). Scan is already >5×. Not official (G1 off).
