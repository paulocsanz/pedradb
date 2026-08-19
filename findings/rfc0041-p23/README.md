# RFC-0041 P1.1/P2.1 attempt — catch-up 25 µs + scan skip empty tombstones

2026-08-18 08:29–08:31. **Load 228 / 188 / 163 on 12 CPUs.** Not official.
Official quiet map remains `findings/rfc0041-p11/head3/`. Peers `sync:false`.

## Code

- `CATCHUP_WINDOW_DEFAULT` 0 → **25 µs** (RFC-0041: raftlog is 16 ops, skip
  threshold 32, so the leader must wait or each client pays one fd). Bound
  `min(window, fd_ema/2)` stays. `PEDRA_CATCHUP_US=0` still disables.
  `catchup_window_knob_roundtrip` asserts 25 µs.
- `count_visible`: skip `range_deleted` when there are no range tombstones
  (YCSB/deps). SST `collect_range_tombstones` returns immediately if empty.

## Grouping

p22 (window 0): `avg_group` 1.19–1.21.  
p23 (window 25 + bound): **`avg_group` 1.38 / 1.42 / 1.41**.

## p23 medians (dirty — do not replace head3)

| shape | p23 med | head3 |
|---|---:|---:|
| ycsb_c | 0.900 | 1.796 |
| deps_scan | 1.674 | 1.790 |
| deps_raftlog_mc4 | 0.562 | 1.792 |
| apply 1c | 0.873 | 1.297 |
| raftlog 1c | 0.346 | 0.994 |

Quiet-box 16×3 still required before the RFC status table can move.
