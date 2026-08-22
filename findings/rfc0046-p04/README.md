# RFC-0046 P0.4 — quiet re-arbiter with the new bounded default (2026-08-22)

Three executions happened under this ID; only the third is official.

1. **v1 — discarded.** Fired at 01:50 (load 8.91) but the three
   official rounds ran at the bench binary's 200-op smoke default
   (`ROCKS_YCSB_OPS` was only exported for the 2M long window), and
   the long window overlapped ~70 s of a concurrent dev run. Fixed in
   `27b1fd7`; see git history of this file for the full v1 account.
2. **v2 — fired clean, measured the dirty tree.** Rounds at the right
   size (n=2000, load 8.76–9.95) — but the build compiled the working
   tree, which carried a parallel session's uncommitted edits
   (occ/memtable/wal/vlog/rocksdb-compat). Detected via the p34
   baseline comparison (below), decomposed by controlled re-runs, and
   demoted to anomaly evidence (`r1..r3/`, `long/`).
3. **clean/ — the official record.** Full battery re-run from a
   worktree at commit `edfa132` (P2.8, no uncommitted edits), same box,
   same quiet bar: 3× g1 / async / kvr at n=2000 with per-round rocks
   pairing, plus the 2M long window 3×.

## Official verdict (clean/, commit edfa132, quiet box)

**The bounded-history default and the entire committed RFC-0046 arc
cost nothing measurable on the parity columns.** Control: the p34-era
tree (`04c7aa2`) re-run today matches `edfa132` on every shape (e.g.
ycsb_e compat 749k/754k vs 700–772k qps; ycsb_a 1.59M vs 1.62M) — the
compat side is stable across the whole arc; the new default is inert
at bench scale by design (wall-clock window; short-lived runs GC
nothing — documented in `docs/usage.md`).

Columns (median-of-3, peer RocksDB default `sync=false` in every
pairing):

- **g1 (official)**: c 4.67, e 2.17, deps_scan 2.04, mvcc 1.54,
  lock_prewrite 0.99; a/f/overwrite at the known `fdatasync` ceiling
  (~0.1 — RFC-0041 P1.2 `todo`, affirmed in code). Same mixed band as
  every prior short window.
- **async**: E 5.88 (5.75/5.88/6.12 — 3/3 ≥5), c 4.83, d 3.76,
  cache_overwrite 3.32, b 3.18, **a 2.98**, lock_prewrite 2.54,
  apply_batch 2.06, scan 2.00, mvcc 1.74, f 1.40, raftlog 0.73.
- **kvr (async)**: scan 35.8, pipeline 5.04, set 4.70, get 3.23,
  mc50 2.13, blob 2.68.
- **long 2M (3×)**: **set 5.45** (5.45/5.18/16.47 — 3/3 ≥5; the 16.47
  is a rocks-depressed run), **pipeline 5.38** (4.95/5.38/6.24 —
  median ≥5, one run at the line), **get 4.71** (4.74/4.71/4.71 —
  3/3 tight under 5: the GET straddle is real, not load).

Renewals that ride this window: RFC-0044 async column (E stays closed
≥5 3/3 at 5.88 vs a peer now ~1.8× faster than Aug 20; SET is now 3/3
≥5 in the quiet long window), RFC-0045 P1.3 (mc50 2.13 med,
lock_prewrite 2.54 med, 3×, no regression), kvr long straddles
(pipeline median ≥5 reached, GET tight at 4.71).

## The v2 anomaly, decomposed (for the parallel session)

v2's async column vs the p34 baseline showed ycsb_a 2.88→0.87 and
ycsb_e 10.5→5.77. Controlled re-runs (2–3× each, tight, same box,
load 8–10) attribute both movements:

| shape (compat, async) | p34 Aug 20 | v2 dirty tree | clean edfa132 | 04c7aa2 today |
|---|---|---|---|---|
| ycsb_a | 1.20–1.42M | 0.48–0.70M | 1.62–1.68M | 1.59–1.61M |
| ycsb_f | 0.55–0.60M | 0.40–0.66M | 0.90–0.92M | 0.80–0.91M |
| ycsb_e | 1.28–1.33M | 1.34–1.37M | 0.70–0.77M | 0.75M |
| ycsb_b | 3.38–3.61M | 4.10–5.15M | 5.19–6.17M | 6.28–6.30M |

- **The committed tree is innocent**: 04c7aa2 ≈ edfa132 on every
  shape, including e.
- **The parallel session's uncommitted edits move compat**: a −2.5×,
  f −1.7× (RMW/update path — occ/memtable/wal are exactly there), e
  +1.8× and b/scan/mvcc/apply +10–30% (plausibly the C2/C3 iterator
  fixes in the RocksDB-API shim). Tight in both directions. **That
  thread should re-check ycsb_a/ycsb_f before committing** — a 2–2.5×
  RMW regression is hiding somewhere in those edits.
- **The rocks peer measured ~1.8× faster than Aug 20** on write-heavy
  ycsb shapes (e: 126k→235k, a: 350k→665k, b: 1.1M→1.75M; tight 3/3
  in both eras, no rocks-side code touched between them). Unattributed
  box/build drift; consequence: current ratios carry a faster
  denominator, which is the honest one to renew against.

Method note for reuse: `${VAR:+...}` expansions in a command's prefix
position are arguments after expansion, not env assignments — export
inside the function (the committed arbiter script does this correctly;
my first clean-battery attempt didn't, and its kvr/long legs failed
fast and were re-run).
