# RFC-0054 P0.2 — deps_raftlog attribution (NON-OFFICIAL box)

`rocks-parity-bench` compat, `ROCKS_YCSB_OPS=2000`, async WAL, 256 MiB buffer.
Same binary, two `ROCKS_PARITY_ONLY` settings. 2026-08-23.

## Discriminator

| run | mem_entries at raftlog enter | batch p50 | qps | p50_ms | max_ms |
|---|---:|---:|---:|---:|---:|
| `ONLY=deps_raftlog` (no seed, no apply) | **0** | 4.3–4.8 µs | **131–159 k** | 5.2–5.7 | 0.18–1.4 |
| full `deps` (seed+apply+scan then raftlog) | **264 192** | 6.4–6.8 µs | **67–76 k** | 7.4–7.8 | 4–6.5 |
| rearm8 official (quiet) | (not logged) | — | **76 k** vs rocks **131 k** | — | 0.063–0.098 |

Isolated Pedra **>1×** vs the rocks official (~130 k). The 0.59× official number is **not** the 16-put encode (build p50 **0.67 µs** both runs).

## What it is

TiKV's raft log lives in a **separate RocksDB** (raftdb). The harness models it as a CF in the **same** Pedra DB. Rocks CFs have **separate memtables**; Pedra prefix-CFs share one. After `deps_apply_batch`, 264k versions sit in the active tail. raftlog inserts pay that occupancy (cache/WAL file, not just `log N` of `tail_idx` — sharding `tail_idx` by CF prefix did **not** move p50).

A fold (`spill_tail`) of those 264k entries untimed before raftlog **ran** (`folded tail=264192`) and **did not** recover isolated p50. Working set of the spilled BTree still occupies the core.

## What it is not

- Submit-path `format!` (0.67 µs)
- Every-8th get (split out of `batch` timer)
- APFS extent stall (rearm8 already killed that; remaining max is box noise)

## Next cut (P0.2 continued)

Per-CF **tails** (not just the index) inside `MemTable`, or a real raftdb-shaped second `Db`. Until then official raftlog stays ~0.6× on a shared memtable. Isolated run is the existence proof that the write core is already faster than Rocks.
