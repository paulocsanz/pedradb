# RFC-0041 P2.1 attempt — host-path CPU cuts + remesure

2026-08-18 07:59–08:01. **Load average 120 / 101 / 88 on 12 CPUs** (fuzzers).
This remesure is **not** the official map. Official quiet map remains
`findings/rfc0041-p11/head3/`. Peers `sync:false` on all 3 runs.

## What landed (host path, tests green)

- `DB::get` / `get_named` — no `ColumnFamily { name: String }` per YCSB-C get
- `count_named` / `last_key_named` — `deps_scan` / latest without handle clone
- `write_owned` — apply/raftlog batches **move** 1 KiB values (was clone in
  `write(&batch)`). `CompatEngine::batch` uses it.
- Tests: `write_owned_moves_values_and_is_durable`, named APIs match handles,
  `deps_suite_on_compat_engine`.

## Median of 3 (dirty box — do not vs head3)

| shape | p21 med | r1 / r2 / r3 | head3 | ≥2.0? |
|---|---:|---|---:|---|
| ycsb_c | 1.470 | 1.47 / 1.32 / 2.43 | 1.796 | no |
| deps_scan | 1.235 | 1.24 / 0.19 / 18.35 | 1.790 | no |
| deps_raftlog_mc4 | 0.532 | 0.44 / 0.90 / 0.53 | 1.792 | no |
| deps_apply_batch | 0.927 | 0.66 / 1.19 / 0.93 | 1.297 | no |
| deps_raftlog | 0.563 | 1.00 / 0.42 / 0.56 | 0.994 | no |
| apply_mc4 | 1.728 | 1.88 / 1.73 / 1.56 | 2.788 | no (load) |
| MVCC | 4.826 | 4.83 / 38.9 / 2.42 | 2.342 | yes (Rocks collapsed) |
| ycsb_e | 0.849 | 1.08 / 0.44 / 0.85 | 2.121 | no (load) |

`qs_*` rows are null (suite not enabled). `avg_group` 1.19–1.21.

**Gate 2.0 on C / scan / raftlog_mc4: not reached.** Re-run when load ≪ ncpu.
