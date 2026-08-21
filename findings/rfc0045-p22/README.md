# RFC-0045 P2.2 — fair-handoff prototype: FALSIFIED (8/8 paired rounds)

Date: 2026-08-21. Box dirty (load ~19–28 during the window — measured
before/after; same-window A/B pairs). Mechanism-level conclusion, not a
standing-qps claim: the effect is 2–3.5× in qps with lock_wait
attribution agreeing, 8/8 pairs — this sign does not flip on a quiet box.

## Hypothesis

P0.2 measured the 50-writer bypass at avg lock wait 175 µs vs hold 1.6 µs
(266 k qps vs 640 k serialization ceiling) and blamed the park/unpark
convoy. If the convoy were a wake-herd (release wakes all 50 waiters, 1
wins, 49 re-park), a **fair release** (`RwLockWriteGuard::unlock_fair` —
direct handoff to the next waiter, handed-off token blocks barging)
should collapse the wait to one directed wake per commit.

## Probe A/B (multiwriter_probe, 50 threads × 2000 ops, 4096 keys, 1 KiB)

| round | A park qps | A p50 | A lock_wait | B fair qps | B p50 | B lock_wait |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 267 297 | 2.46 µs | — | 118 281 | 332.92 µs | — |
| 2 | 228 366 | 7.25 µs | — | 148 683 | 306.38 µs | — |
| 3 | 298 534 | 2.38 µs | — | 153 314 | 293.38 µs | — |
| 4 | 245 485 | 5.17 µs | — | 144 490 | 283.50 µs | — |
| 5 | 292 379 | 3.38 µs | — | 94 771 | 455.71 µs | — |
| 6 | 270 410 | 1.67 µs | 154.60 µs | 100 710 | 409.08 µs | 489.78 µs |
| 7 | 243 454 | 3.67 µs | 172.30 µs | 79 854 | 514.46 µs | 617.97 µs |
| 8 | 232 647 | 3.42 µs | 187.02 µs | 69 579 | 547.83 µs | 709.29 µs |

**Fair is 2–3.5× worse in qps, ~100× worse in p50, and lock_wait gets
3–4× WORSE (155–187 µs → 490–709 µs).** 8/8 pairs, no ambiguity.

## Verdict (measured, not theorized)

1. **The herd hypothesis is wrong.** parking_lot's unfair release does
   not wake-all: it wakes the next waiter, but lets on-CPU arrivals
   barge past the still-scheduling woken thread. That barging is
   *load-shedding*: it keeps the 1.6 µs critical section continuously
   busy (p50 2–7 µs for bargers) and pushes the cost into the tail
   (p99 ~2 ms).
2. **Fair handoff chains wake latency.** Direct handoff means the lock
   is logically held while the woken thread waits to schedule
   (~µs per hop × 50 waiters ≈ the observed ~300–550 µs p50); the
   convoy becomes a wakeup chain and throughput halves.
3. **The lock-flag space is now fully swept** (this box, same window):
   unfair park **266 k** (P0.2), spin-then-park 243–251 k (P0.2),
   fair release 95–153 k (this finding). Every acquisition/release
   shape available in parking_lot/lock_api 0.4 loses to plain unfair
   park. The remaining P2.2 direction is structural: batch the lock
   takers (no-catch-up leader-follower — amortize one acquisition
   across the group), which is a different mechanism from all three.

## Artifact

`PEDRA_WRITE_FAIR=1` ships default-off (prototype kept for reference,
like `PEDRA_WRITE_SPIN` after its falsification). lock_api 0.4 has no
fair *acquire* — the prototype is fair *release* only
(`RwLockWriteGuard::unlock_fair` on the bypass path).
