# P1.3 kvrocks_get — TLS last-get cache redesign (2026-08-22)

RFC-0044 **P1.3** (`kvrocks_get` ≥ 5× vs RocksDB default `sync=false`).
Entry point: the P0.4 clean battery measured GET 4,74/4,71/4,71 (3× tight,
quiet 2M) — a real ~5% gap, not load. This is the engineering follow-up:
profile first, then fix.

All numbers experiment-grade (single box, load ~12 during runs, back-to-back
A/B on identical config — relative deltas only). Official quiet 3× re-run
armed separately.

## Profile (macOS `sample`, 1 ms, worktree at `c1720de`, 200M GET ops)

Bench loop internals (`run_kvrocks` GET window, `get_probe` →
`rocksdb_compat::DB::contains`):

| component | share of loop | note |
|---|---|---|
| `Instant::now()` + `ms(t)` (harness) | ~61% | identical for both engines — the ratio ceiling at engine→0 is ~7× |
| `get_probe` engine work | ~36% | **~72% of that is the `AnswerCache` fallback walk** |
| other | ~3% | |

The TLS `LastGetTable` was **missing ~85% of the time** (1600 samples through
`ConcurrentDb::get` → `AnswerCache::get` vs ~276 through `LastGetTable::get_key`),
despite the official shape being 1024 uniform keys, no writes in the window.
Cause: 1024 slots, 2-slot linear probe, load factor 1.0 — collided keys
ping-pong evict each other (`store_key` blind-evicts probe-1 when both probes
hold different keys). Instrumented fixed build: **95.6% hit rate**
(47 799 471 hits / 2 200 529 misses, 50M ops).

Secondary finding: `prepare()` cleared **all 1024 slots on every epoch
change**, and `read_cache_epoch` bumps on **every published write** — so every
read-after-write in mixed workloads (ycsb_a/f RMW) paid an O(1024) clear.
Removing it is worth more than the hit-rate fix on those shapes.

## The fix (`crates/rocksdb-compat/src/lib.rs`)

1. **Lazy per-slot epochs**: each `LastGetSlot` stores its epoch; a slot whose
   epoch ≠ the reader's is stale (miss, probe continues). `prepare()` deleted —
   invalidation is now O(1) instead of an O(N) clear per published write.
2. **Capacity**: `LAST_N` 1024 → 2048, `LAST_PROBE` 2 → 8 (load 0.5; probe-4
   measured 92.2% on the deterministic sequential fill, probe-8 passes the
   95% floor). Stale-slot-preferred insertion: a live entry is evicted only
   when all 8 probes are live.
3. **`get()`/`contains()` share one TLS table** (identical default-CF query;
   previously the bench's warm phase filled a different table than the timed
   one).
4. **`cache_epoch_base` (fix C1/C1b) is required**: `read_cache_epoch` starts
   at 1 in every instance, so with lazy epochs a second DB instance in the
   same thread could epoch-match the first instance's entries (adversarial
   campaigns fail: `pre-crash silent-wrong`). The redesign ships with the
   process-wide `CACHE_ID << 32` base — same mechanism as the parallel
   session's uncommitted fix C1/C1b (RFC-0048 tree); text converges.

## A/B (worktree `c1720de`+fix vs `c1720de`, back-to-back, async WAL)

| shape | base qps | fixed qps | Δ |
|---|---|---|---|
| kvrocks_get 200M (uniform 1024) | 11 058 071 | 15 795 598 (probe-8) | **+43%** |
| kvrocks_get 2M (official window) | 11 502 221 | 14 912 556 | +30% |
| ycsb_a 1M zipf | 1 779 522 | 2 682 996 | **+51%** |
| ycsb_c 1M zipf | 1 648 103 | 2 580 694 | **+57%** |
| ycsb_f 1M zipf | 1 666 875 | 2 687 454 | **+61%** |

ycsb_a/f gain mostly from the O(1) invalidation (no more 1024-slot clear per
read-after-write). RocksDB peer is ~400 ns/GET on this box → projected GET
ratio at the official window: 400/67 ≈ **6.0×** (was 4.71). ycsb A/F wall
ratios move proportionally (p22 follow-up).

## Tests

- `last_get_table_lazy_epoch_invalidation` — stale entry never answers for a
  new epoch; re-store answers without a clear; old-epoch entries coexist
  without leaking.
- `last_get_table_keeps_uniform_working_set` — deterministic 1024-key
  sequential fill ≥95% hit (probe-8 passes; probe-4 measured 944/1024).
- `tls_get_cache_never_answers_across_instances` — two DBs, one thread, same
  key: no cross-instance answer before any epoch bump (the adversarial
  campaign scenario as a fast unit).
- Existing TLS invalidation/zipf tests (36) + adversarial suite (7) green at
  the combined tree and at the standalone commit tree.

## Limitations

- Experiment-grade: ONLY-filtered runs, dirty box (load ~12), single machine,
  `StdEnv` temp dir. Official quiet 3× arbiter re-armed separately.
- The harness (`Instant::now()` ×2 + lat push ≈ 55 ns/op) bounds the ratio at
  engine-cost 0 to ~7× — noted, not changed (both engines measured
  identically; touching the harness is not an honest lever).
- `get_named`/`count_named` tables keep the old table-level clear (correct,
  unchanged); only their epoch lines gained `cache_epoch_base`.
