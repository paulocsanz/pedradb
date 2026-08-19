# RFC-0041 P0.3 — FLOOR=2.0 on shapes that already pass

2026-08-18. P0.3 is the **opt-in** recipe, not a default that fails
head3 run1.

head3 per-run (`compat_over_rocksdb`) for the median-passers:

| shape | r1 | r2 | r3 | mediana | 3/3 ≥ 2.0? |
|---|---:|---:|---:|---:|---|
| apply_mc4 | 1.301 | 2.819 | 2.788 | 2.788 | não |
| MVCC | 2.389 | 2.342 | 1.620 | 2.342 | não |
| ycsb_e | 2.693 | 2.121 | 0.932 | 2.121 | não |

Defaulting `FLOOR=2.0` on those three makes `tikv_ycsb_parity_v0.sh`
exit 2 on a quiet official run (run1 apply_mc4, run3 MVCC/E). The
script therefore **documents** the env and leaves FLOOR unset on
`SYNC=0`. Compare still lists all 16.

Opt in:

```
ROCKS_PARITY_RATIO_FLOOR=2.0 \
ROCKS_PARITY_GATE_SHAPES=deps_apply_batch_mc4,deps_mvcc_latest,ycsb_e \
  scripts/tikv_ycsb_parity_v0.sh
```

P2.3 (floor on the full set) stays off: 1c write 2× Rocks async is
above one `fdatasync` (`rfc0041_one_fdatasync_cannot_hit_2x_rocks_default_ycsb_a`).
This session’s box was load ~78 — no new 16×3; map remains head3.
