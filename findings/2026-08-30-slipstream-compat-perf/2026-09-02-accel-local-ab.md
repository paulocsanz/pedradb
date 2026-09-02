# Block-index accel local A/B (285af59 vs e0b38f5) — 2026-09-02

Setup: single-variable A/B via git worktrees; stage [patch] pointed at each
worktree's rocksdb-compat (path deps pull the whole pedradb tree). Binaries
frozen before the next build (cargo overwrites the same hash-named file).
CTL = e0b38f5 (= guest v41p code state), ACC = 285af59 (accel only).

## 25M/256MiB attempt — INVALID for point legs (recorded for the ledger)

get_hit medians swung 79-493µs across rounds; rocks itself moved 80->204µs.
With a ~5GiB DB vs 256MiB block cache, successive hydrations thrash the OS
page cache and every arm is IO/cache-state-bound. Same for lookup_100
(70-520µs/get). Local 25M point legs cannot arbitrate code changes — use
4M/1GiB (hot, CPU-bound) or the guest. Matches the parked read-legs-25m note.

## 4M entries / 1GiB cache (hot, CPU-bound), 3 interleaved rounds

pedra medians (µs), [lo med hi] condensed to med:

get_hit:      ctl 4.55(r1, warming) 2.78 2.91 | acc 2.66 2.38 2.19
get_loop x100: ctl 328.2 342.9 282.7          | acc 218.0 206.7 205.6
multi_get x100: ctl 339.6 404.4 305.6         | acc 228.3 213.1 208.5

Hot-round deltas (r2/r3): get_hit -15..-25%, get_loop -27..-40%,
multi_get -32..-47%. 9/9 pedra round-pairs favor ACC. Rocks drift over the
same rounds is <=5% (2.81->2.58 get_hit) — the delta is code, not machine.
Hot acc pedra also beats rocks: get_loop 206 vs 248, multi_get 213 vs 224
(r3), get_hit 2.19 vs 2.58.

Reference (same machine, clean run earlier tonight, 25M): prefix_scan
pedra 146.42 vs rocks 219.93 = 1.50x local, diag-off.

## Verdict

KEPT (was already committed as 285af59). Guest injection v42 staged and
launched (table.rs 88b51fc2 -> 53cbcf56, one variable, diags unchanged);
compile signature verified (Compiling pedradb-core/rocksdb-compat/
beyond-slipstream in serial). Expected guest effect is bounded by the
~40µs/probe IO floor, so the guest capture is a no-regression check +
ledger row, not a large ratio mover.
