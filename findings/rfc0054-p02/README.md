# RFC-0054 P0.2 — deps_raftlog attribution (NON-OFFICIAL box)

`rocks-parity-bench` compat, `ROCKS_YCSB_OPS=2000`, async WAL, 256 MiB buffer.
Same binary, `ROCKS_PARITY_ONLY` splits + per-shape phase deltas
(`write_phase_snapshot`, added for this cut). 2026-08-23.

## Discriminator (before any fix)

| run | mem_entries at raftlog enter | batch p50 | qps | raftlog-only publish |
|---|---:|---:|---:|---:|
| `ONLY=deps_raftlog` (no seed, no apply) | **0** | 4.3–4.8 µs | 131–159 k | 0.54–0.57 µs |
| full `deps` (seed+apply+scan then raftlog) | **264 192** | 6.4–6.8 µs | 67–97 k | **4.20 µs** |
| rearm8 official (quiet) | — | — | 76 k vs rocks 131 k | — |

## The real mechanism (phase deltas corrected the first theory)

First attribution blamed the shared memtable tail (264k versions) — wrong.
The cumulative `write_phase_line` averaged ALL commits of the run, so apply's
fat batches diluted it: raftlog-only deltas show **mem = 2.6 µs in BOTH runs**
(the tail was never the cost). The loss is the **publish** phase:

| phase (raftlog-only) | isolated | full deps |
|---|---:|---:|
| prepare | 0.18 µs | 0.18 µs |
| wal | 1.22 µs | 1.53 µs |
| **mem** | **2.63 µs** | **2.67 µs** |
| **publish** | **0.57 µs** | **4.20 µs** |

`publish_sequence` → `invalidate_read_answers` → `count_cache.record_dirty`:
once `deps_scan` fills the count cache with windows, EVERY publish allocates
**2 `Box`es per written key** (dirty `order` + `by_key`) — 32 allocs per
raftlog commit — plus the overflow `retain`. Isolated raftlog never counts,
the entry map stays empty and `record_dirty` early-returns. That is why the
fold-before-raftlog probe (tail emptied, p50 unchanged) and the CF-sharded
`tail_idx` moved nothing: neither touches the count cache.

## Fix (this branch)

`CountCache` envelope + insert watermark:

- **Envelope** `env_lo`/`env_hi` (min start / max end of cached windows,
  sticky through eviction, reset on `clear`): a published key outside it
  cannot invalidate any present entry — no dirty-log allocation.
- **Watermark** `dropped_below_max`: an answer computed before a dropped
  publish cannot know if a dropped key was inside its window (F204 in-flight
  reader), so `insert` refuses to cache below the watermark. No get-time
  check, no per-prefix buckets (a first draft retired same-prefix windows it
  shouldn't — `count_cache_invalidates_only_overlapping_writes` caught it).

Also fixes **F219** found on the way (pre-existing on `main` via the CF-shard
commit): `tail_idx_range` returned an EMPTY range for cross-shard bounds
(`["u/03","u/04")`), so `last_visible_under_prefix` dropped all tail keys of
the prefix — `last_under_prefix_versions_and_tombstone` + 3 more red on
`main` since ae89515. Now an exact multi-shard iterator (order-agnostic;
`MemInternalIdx` keeps its sorted single-shard guard).

## After the fix (same noisy box)

| run | batch p50 | raftlog-only publish | qps |
|---|---:|---:|---:|
| isolated | 4.83 µs | 0.54 µs | 152 k |
| full deps | **5.29 µs** | **0.90 µs** | 94–97 k (max_ms outliers 4–112 ms; box at 16 Gi free disk, quiet battery decides) |

Per-CF tail vecs: **disproven** — mem phase is flat. Second raftdb-shaped
`Db`: unnecessary for >1×.
