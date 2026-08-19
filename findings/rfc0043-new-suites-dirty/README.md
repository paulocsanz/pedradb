# RFC-0043 — kvrocks + myrocks + surreal (DIRTY BOX)

**Not official.** Not median-of-3. Do not compare to head3.

- Mix: 4096 / 2000 / 1 KB / zipfian / `ROCKS_DEPS_BATCH=32`
- Pedra: always fdatasync before Ok
- `run1/`: all Rocks writes async (wrong for MyRocks/Surreal)
- `run2-hostdefault/`: **host default** — Kvrocks async; MyRocks
  `flush_log_at_trx_commit=1`; Surreal `sync=every`. JSON
  `peer_policy=host-default`. Load ~80 / 12 CPUs.

- `run3-kvrocks/` / `run3-surreal/`: isolated remesure after OCC
  commit uses write-group (`fdatasync` off the write lock) + point
  `key_has_write_after`. Load ~110 / 12 CPUs. Not official.

JSON: `run3-kvrocks/compare/` and `run3-surreal/compare/`.

## run4 + P2.5 code (2026-08-18) — still dirty, sanity only

- `run4-*/`: extra `flush()` after seed (get 1.99× but set/pipeline
  regressed — every later SET hit a fresh mem+SST). **Reverted**;
  untimed full-keyspace warmup gets kept.
- P2.5 CPU cuts (in this tree, RFC-0043 P2.5): `get_at` fast-path via
  point cache when `snap.seq == published_seq` (double-checked);
  `apply_batch_occ` validates write-set by reference, walks read-set
  only when a publish raced; `kvrocks_set_mc50` (redis-benchmark
  default `-c 50`) added — 1-client `kvrocks_set` stays as the G1
  canary (1 fd per Ok; 0.80 vs async peer is physically out of reach,
  same class as RFC-0041 canaries 0.23–0.57).
- Sanity (tiny 512/100/64B, load 184!): `kvrocks_set_mc50` 0 errors,
  write_group submits=5712 groups=920 avg_group=6.21 (vs 1.0
  1-client) — group commit absorbs the fd at concurrency.
- `run5-kvrocks/` / `run5-surreal/`: isolated remesure after P2.5 code
  (OCC ref-validate + `get_at` cache fast-path + warmup-only seed).
  **DIRTY: load 184→234 on 12 CPUs.** Direction only, not official:

  | shape | run5 ratio | run4 | run3 | target |
  |---|---:|---:|---:|---|
  | kvrocks_get | 1.611 | 1.99 | 0.44 | >1.0 ✓ |
  | kvrocks_scan | 66.09 | 7.44 | 26.3 | >1.0 ✓ |
  | kvrocks_pipelined_set | **0.813** | 0.47 | 1.23 | ≥0.80 ✓ (recovered) |
  | kvrocks_set (1c) | 0.116 | 0.14 | 0.48 | canário físico (1 fd/Ok) |
  | kvrocks_set_mc50 | 0.373 | — | — | needs quiet remesure (group 6.21 ops/fd under load) |
  | surreal_tx_get | 415.5 | 222 | 142 | >1.0 ✓ |
  | surreal_tx_scan | 11.87 | 26.4 | 88.9 | >1.0 ✓ |
  | surreal_tx_put | **1.065** | 0.88 | 0.78 | ≥0.80 → **>1.0 ✓** |
  | surreal_tx_batch | **1.537** | 0.90 | 1.23 | ≥0.80 → **>1.0 ✓** |
  | surreal_tx_rmw | 0.593 | 0.74 | 1.76 | ≥0.80 open — peer anomalous this run (rmw 5009/s > its own put 3072/s is incoherent; rmw ⊃ put), compat side is fine (rmw 2973 ≈ put 3273) |

- **Quiet-box isolated remesure (kvrocks + surreal, 4096/2000/1KB
  zipf, peer = host default) still pending — load never dropped.**

## run6 (2026-08-19) — P2.6 code, still dirty (load 126–184)

Catch-up `active≥16` → 1× fd_ema; `peer_anomalies`; new catalog
shapes; `surreal_tx_rmw_mc8`; `kvrocks_blob_set`.

**Surreal (no peer anomaly this run — rmw 3536 < put 4293):**

| shape | run6 |
|---|---:|
| surreal_tx_get | 270 |
| surreal_tx_scan | 69.8 |
| surreal_tx_put | **1.507** |
| surreal_tx_rmw | **1.114** (meta 0.80→>1.0) |
| surreal_tx_batch | **1.333** |
| surreal_tx_rmw_mc8 | 0.269 (OCC conflict; 0 errors) |

**Kvrocks (async peer; write_group avg 6.41 ops/fd under load 184):**

| shape | run6 |
|---|---:|
| kvrocks_get | 1.481 |
| kvrocks_scan | 25.4 |
| kvrocks_pipelined_set | 0.470 |
| kvrocks_set 1c | 0.018 (canário G1 vs async; load crushed Pedra to 945 qps) |
| kvrocks_set_mc50 | 0.372 (same as run5; group 6.4 — scheduler-starved) |
| kvrocks_blob_set | 0.197 (16 KiB, 1 fd/Ok) |

**Expanding catalog (7 suites, one LSM — write pollution; not isolated):**

| shape | run6 | note |
|---|---:|---|
| nebula_get_neighbors | 2.601 | |
| nebula_insert_edge | 0.327 | batch write vs async |
| flink_window_state | 0.708 | |
| kafka_changelog_flush | 13.62 | Rocks flush 17 qps this run |
| bluestore_omap_write | **1.026** | host-sync; fair |
| bluestore_omap_read | 13.05 | |
| solana_shred_append | 0.367 | |
| solana_trailing_read | 1.070 | |
| arango_doc_crud | 0.039 | polluted (puts after fat write storm) |
| arango_traversal | 4.754 | |
| venice_fanout_get | 1.456 | 32 gets/op |
| rockstore_widecol_rw | 0.366 | |
| oxigraph_spo_lookup | 1.101 | Oxigraph still RocksDB |
| oxigraph_triple_put | 0.370 | |

Raw: `run6-kvrocks/`, `run6-surreal/`,
`run6-nebula,streaming,ceph,solana,arango,venice,oxigraph/`.
Not official.

## run7 (isolated suites, load ~65–75) + run8 (OCC group commit)

run7 isolated the polluted expanding catalog. run8: OCC commits join
the write group (validate on the leader); begin is lock-free under
contention; rmw_mc8 retries 32×. Load ~41–44.

**Surreal run8 (0 errors, no peer anomaly):**

| shape | run6 | run7 | run8 |
|---|---:|---:|---:|
| put | 1.51 | 0.86 | **1.078** |
| rmw | 1.11 | 0.79 | **0.926** |
| batch | 1.33 | 0.84 | 0.849 |
| rmw_mc8 | 0.269 | 0.119 (34 err) | **0.650** (0 err, 7426 vs 11432 qps) |
| get / scan | 270 / 70 | 40 / 8.3 | 138 / 10.9 |

**Kvrocks run8** (avg_group 7.14):

| shape | run6 | run7 | run8 |
|---|---:|---:|---:|
| get | 1.48 | 2.12 | 1.61 |
| set 1c | 0.018 | 0.113 | 0.179 (canário G1) |
| pipeline | 0.47 | 0.43 | 0.576 |
| set_mc50 | 0.372 | **0.648** | **0.636** |
| blob_set | 0.197 | 0.170 | 0.538 |
| scan | 25 | 8.2 | 4.48 |

**Expanding catalog run7 (isolated, vs run6 one-LSM):**

| shape | run6 mixed | run7 isolated |
|---|---:|---:|
| arango_doc_crud | 0.039 | **0.216** (still 30% 1-put/fd vs async) |
| arango_traversal | 4.75 | 3.30 |
| nebula_get_neighbors | 2.60 | 0.65 |
| nebula_insert_edge | 0.33 | **0.931** |
| flink_window_state | 0.71 | 0.175 |
| kafka_changelog_flush | 13.6 | 9.43 |
| bluestore_omap_write | 1.03 | **1.192** (host-sync, fair) |
| bluestore_omap_read | 13.1 | 4.62 |
| solana_shred_append | 0.37 | 0.107 |
| solana_trailing_read | 1.07 | 3.00 |
| venice_fanout_get | 1.46 | **1.784** |
| rockstore_widecol_rw | 0.37 | 0.328 |
| oxigraph_spo_lookup | 1.10 | 0.624 |
| oxigraph_triple_put | 0.37 | **1.001** |

Quiet 3× still pending. rmw_mc8 0.65 and set_mc50 0.64 are the
contention leftovers — group is 7 ops/fd under load 40.
