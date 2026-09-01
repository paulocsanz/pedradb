# Local 25M read-leg A/A: guest gaps do not reproduce locally

Date: 2026-09-01. Machine: local mac (12 cores, 96 GiB RAM, APFS; load 5–7 from
the concurrent session throughout). Stage bench `/tmp/slip-inject/stage`
(criterion `--bench 'get_hit|prefix_scan|lookup_100'`,
`SLIPSTREAM_BENCH_ENTRIES=25000000`, default caches: rocks 1 GiB hyper-clock
uncompressed-block cache / pedra 1 GiB payload pool, both backends one process).
Tree = HEAD + concurrent session uncommitted state; image-equivalent v33.
Captures: `local-25m-readlegs-run1.log`, `local-25m-readlegs-run2.log`.

Ratios = rocks/pedra (≥1.0 = pedra faster):

| leg                | run1  | run2  |
|--------------------|-------|-------|
| get_hit            | 1.414 | 1.321 |
| prefix_scan        | 1.182 | 1.055 |
| lookup_100 get_loop| 1.313 | 0.795 |
| lookup_100 multi_get | 1.313 | 1.399 |

Reads on this machine are latency-bound at ~85–130 µs/get for BOTH backends
(pedra get 87–90 µs vs rocks 115–127 µs at 25M; probes p50 117 vs 138 µs) —
i.e. every access faults through the storage stack even though RAM would hold
the fold; CPU-side differences are second-order locally. The guest gaps
(get_hit 0.80×, prefix_scan 0.67×, get_loop 0.75×, multi_get 0.79×, #20–#34
era) do NOT appear in this regime; pedra leads most legs.

Run 2 get_loop 0.795× is a single-leg anomaly: 14.98 ms vs 8.53 ms (run 1)
and 8.61 ms for multi_get in the SAME run 2 process seconds later — a ~6 s
interference burst during that one leg, consistent with external load, not a
distinct code path (get_loop and multi_get share the get path; multi_get is a
sequential loop of gets).

Conclusion / next: local ratios carry no iteration signal for the guest gap.
Next number: guest baseline at image v33 with BOTH backends in one process
(the boot bench currently runs pedra-diag only) to establish whether the four
gaps still exist on current code before choosing levers. probe_miss remains
behind locally (667 ns vs 292 ns p50 — linear 86-SST walk vs rocks' level
binary search), but probe legs are not in this goal.
