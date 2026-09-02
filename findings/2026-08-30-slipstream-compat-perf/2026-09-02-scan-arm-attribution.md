# Scan arm attribution — local clean run + guest excess decomposition (2026-09-02)

Context: guest v41+PAGEDIAG run1 prefix_scan pedra 404.72µs vs rocks 353.39µs
(ratio 0.873, diags ON). Local 25M/256MiB clean run (no diags): pedra 165.07µs
(diag-on run), rocks 208.26µs. Standing question: where does the guest pedra
excess (+124µs vs rocks-scaled) live, and which lever attacks it.

## Method

macOS `sample` 3x5s during the same clean local 25M run, one sample straddling
the rocks arm (sample-scan-rocks-1), two straddling the pedra arm
(sample-scan-rocks-2/3). Leaf self-time classified by symbol under each arm's
`scan_prefix` root (self = count - sum(children); column-of-count stack parse).
Scripts: `classify_subtree.py`, `classify_pedra.py` (session scratch).

NOTE: `sample` collapses inlined closure variants under misleading
`db_iterator...next` symbols; classification must be by leaf symbol, not by
frame name. Root children sum exactly to the root inclusive count (2322).

## Local split (arm-share, same run)

rocks arm (2322 arm samples):
- iterator-own (C++ Next chain + prefetch fcntl + perf_context): 72.0%
- shared harness (memmove/malloc/decode_entry/from_utf8/collect): 21.1%
- tlv_get_addr (TLS reads, mostly perf_level/perf_context): 6.8%

pedra arm, diag-off (1049 arm samples):
- core (StreamingVisibleIter 43.2% incl SstRangeIter::next 17%, next_window_kv 17%)
- compat (page_forward_inner + decode_bytes): 9.9%
- shared harness: 27.8%
- Bytes refcount atomics (shared_drop + shared_clone + promotable_even_clone): 10.9% (~18.0µs/op)
- swtch_pri (sched/lock): 5.0%

Cross-check: shared harness is EQUAL in both arms in absolute µs
(rocks 0.211*208.26=43.9µs; pedra 0.278*165.07=45.9µs) — harness work per row
is shared in kind and cost, as designed (same closures per row).

## Guest scaling

rocks arm scales UNIFORMLY at 1.697x to guest:
- own 149.9 -> 254.8µs, harness 43.9 -> 74.6µs, TLS 14.2 -> 24.1µs; sum 353.5
  vs measured 353.39. The whole rocks op is machine-generic 1.70x.

pedra guest (clean est. ~388µs after removing 13-20µs diag tax):
- harness ~74.6µs (shared, scales like rocks)
- excess vs 1.70x scaling: +124µs/op total, of which:
  - page_forward internal: guest 94.3µs vs 33.3 local = 2.83x (+38µs over 1.70x)
  - outside page: guest ~310µs vs 131.5 local = 2.36x (+87µs over 1.70x)

## Key conclusions

1. The in-page per-row degradation (rows 172-201ns guest vs 53-56 local, 3.3x)
   contains NO Bytes refcount atomics (page build is zero-copy offsets since
   d70e710; block-cache atomics ~0.13/row). Atomics cannot explain in-page
   excess. The page path is varint-decode + compare + push — branchy compute.
2. Outside-page excess (+87µs) decomposes into: Bytes atomics (local 18µs,
   degrades unknown), swtch/lock (local 8.3µs), core/compat outside-page glue
   (local ~61µs), setup (guest 4.8-6.1µs vs local 0.22µs).
3. Guest CPU (x86, mitigations) penalizes pedra's branch-serialized per-row
   code 2.4-3.3x while rocks' memmove+virtual-heavy C++ degrades exactly 1.70x.
   Strongest hypothesis: uarch sensitivity of small branchy loops, not a single
   hidden tax. Implication: parity on scan needs LESS WORK PER ROW
   (algorithmic), not just removing a constant.
4. Structural per-row gap: pedra merges 2 streams/op (SCANDIAG streams/op=2.0)
   and runs a second compat decode (decode_bytes ~2.4%) per row; the rocks
   sample shows a single LevelIterator under MergingIterator and no second
   decode pass. ~15-25µs/op guest-equivalent of extra compare+dispatch+decode
   per row that rocks does not pay.

## Ranked scan levers (post-attribution)

a. Single-stream fast path in StreamingVisibleIter when only one live run
   overlaps the prefix (skip merge compare per row) — core (mine), correctness-
   neutral, targets conclusion 4. Est guest 10-25µs/op.
b. Bytes-atomics cut in compat iterator (mine: lib.rs/iter_kernel.rs) — kills
   ~2-3 atomic ops/row; local 18µs/op. Guest effect unknown (atomics may be
   sub-linear vs 1.70x); discriminate with (a) first since (a) is bigger.
c. Box<dyn Iterator> -> concrete enum for run streams — removes ~666 indirect
   calls/op; est 10-27µs/op guest IF retpoline-class costs apply. Verify guest
   mitigations before investing.
d. Setup 5µs anomaly (22x local) — 1.2% of op, parked.

Not levers: per-row Vec in page (REFUTED, zero-copy), harness closure
(theirs + identical cost in both arms), lock span (measurement-only diag).
