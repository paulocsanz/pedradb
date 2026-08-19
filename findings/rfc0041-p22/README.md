# RFC-0041 P2.2 attempt — lock-free get/count cache on ConcurrentDb

2026-08-18 08:15–08:18. **Load 232 / 175 / 138 on 12 CPUs.** Not official.
Official quiet map: `findings/rfc0041-p11/head3/`. Peers `sync:false` ×3.

## Code (this is the slice)

`ConcurrentDb::get` already answered point-cache hits without the Db
`RwLock`. The harness never called it: `DB::get_named` used
`with_read(|db| db.get())`, so **every YCSB-C get took the read lock**.
Now `get_named` → `inner.get()`.

`count_cache` is `Arc` like the point cache. `ConcurrentDb::count_in_range`
hits it without the Db lock; `count_named` (`deps_scan`) uses that.
`count_in_range_cache_matches_locked_and_invalidates` asserts fill /
hit / invalidate after put.

## p22 medians (dirty — Rocks C 1.41 M → 666 k)

| shape | p22 med | runs | head3 | ≥2.0 official? |
|---|---:|---|---:|---|
| ycsb_c | 2.405 | 0.58 / 2.41 / 2.71 | 1.796 | **no** (peer starved) |
| deps_scan | 1.618 | 7.12 / 1.28 / 1.62 | 1.790 | no |
| deps_raftlog_mc4 | 0.701 | 0.87 / 0.70 / 0.33 | 1.792 | no |

Do not promote p22 into the RFC status table as a 2.0 close.
