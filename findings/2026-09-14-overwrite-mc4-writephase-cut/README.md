# Detector: overwrite_mc4 / ycsb_f_mc4 WRITEPHASE → cut=wal_write

**When:** 2026-09-14. **Host:** Darwin DIAG. **Not** cartaz.
**Harness:** `rocks-parity-bench` `run_clients` async vs async
(`PEDRA_PARITY_ASYNC=1`, `ROCKS_PARITY_SYNC=0`), 4 clients, 100k zipf.
**Detector:** `write_cycle_kernel::name_cut` (RFC-0192) on the 7-tuple
PHASE the bench already emits (`PEDRA_WRITE_PHASE_STATS=1`).

## overwrite_mc4 (200k ops)

```
wgΔ submits=200000 queued=165182 groups=143863 ops=200000 avg_group=1.39
phasesΔ prepare=0.99µs wal=20.80µs mem=0.68µs publish=0.16µs
        flsh=0.00µs lock_wait=0.41µs n=143863
        cw=0.00µs/grp lone[n=0]
```

`name_cut` = **wal_write** (20.80µs = **91.9%** of CS).
`lock_wait` is derived, not a cut (0.41µs). Flush is zero.
Lone path is dead (`n=0`) — rmw_sched ON is in the group path.
`avg_group=1.39`: drain-what's-queued, no collect window, so almost
one `write()` per op.

Linux quiet pin (0189 P0.1) was `wal_write=890ns` `cut=mem_guard`.
This Darwin slice is **23×** more WAL syscall than that pin.

`write_cycle_forecast` (lumped mem as `mem_guard`):
- cs=22.63µs cycle=23.04µs qps_hat=43k/group ×1.39 ≈ 60k
- if `write()` left the mutex: qps_hat 446k (same 1 write/group)
- if avg_group=4 amortized wal: qps_hat 134k

Railway 0.35–0.44 vs Rocks ~280k is this: **WAL `write()` per tiny group**.

## ycsb_f_mc4 (puts half of 200k ops)

```
wgΔ submits=99797 queued=70212 groups=74029 ops=99797 avg_group=1.35
phasesΔ prepare=1.22µs wal=21.51µs mem=0.67µs publish=0.48µs
        lock_wait=1.04µs n=74029 lone[n=0]
```

Same cut: **wal_write 90.1%** of CS. Gets are extra on top of this put tax.

## Not the owner

- leftover / DONTNEED (hot 24 MiB)
- scan readahead
- `lock_wait` / park convoy
- flush
- grouping *off* (it is on; groups are just size ~1.4)
