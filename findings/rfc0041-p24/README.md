# RFC-0041 — fat-batch catch-up waits the full window

2026-08-18 08:43–08:46. **Load 107 / 134 / 153 on 12 CPUs.** Not official.
Official map: `findings/rfc0041-p11/head3/`. Peers `sync:false`.

## Policy (tested)

`catchup_wait_bound(..., batch_ops)`:

- 1-op: still `min(window, fd_ema/2)` (RFC-0042; `catchup_wait_bounded_by_half_fd`)
- `batch_ops >= 16` (raftlog): **full window**, not fd/2 — a sibling 16-op
  client saves a whole `fdatasync`
- apply 64 ops still skips (`CATCHUP_SKIP_OPS`)

`catchup_bound_policy` asserts both branches.

`count_visible` min-head no longer calls `head()` twice per cursor.

## p24 (dirty)

`avg_group` 1.36 / 1.36 / 1.42 — same class as p23 (25 µs + fd/2). Load
prevents siblings from arriving even with a longer fat-batch wait.

| shape | p24 med | head3 |
|---|---:|---:|
| ycsb_c | 0.469 | 1.796 |
| deps_scan | 1.042 | 1.790 |
| deps_raftlog_mc4 | 0.250 | 1.792 |

RFC status table **not** updated.
