# RFC-0045 P0 — multi-writer profile (bypass path) + lock_prewrite context

Date: 2026-08-20, 18:51–19:06. Box: fuzzers dead (user-killed earlier);
`caixote-api` + cargo-watch for other projects resident. Load during runs
9–20 (recorded per run below) — **dirty box**; these are *mechanism*
numbers (relative in-process attribution), not standing qps claims.
Árbitro para números em pé continua sendo P2.1-quiet (<10, 3 rounds).

## P0.1 — instrumentation shipped

`PEDRA_WRITE_PHASE_STATS=1` (opt-in at open; `None` = one branch/commit):
`WritePhaseStats{commits, prepare_ns, wal_ns, mem_ns, publish_ns,
flush_check_ns, lock_wait_ns}` atomics; `commit_async_ops` attributes its
five phases, the `ConcurrentDb` bypass attributes RwLock write-acquire
wait. Probe: `crates/pedradb-core/examples/multiwriter_probe.rs`
(barrier-aligned N threads, skewed keys over 4096, 1 KiB values,
`auto_flush_bytes=1 GiB` so the window is flush-free — default 4 MiB
flushes every ~4k ops and swamps the phases).

## P0.2 — where the writer's ns go (bench-shaped window: 50×2000 ops)

Load ~19 during this sweep (hot). Relative attribution is the signal.

| threads | qps | p50 | p99 | lock_wait avg | prepare | wal | mem | publish |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1.05 M | 0.67 µs | 10.5 µs | 0 | 0.04 | 0.37 | 0.14 | 0.04 |
| 50 (park) | 266 k | 11–12 µs | 1.2 ms | **175 µs** | 0.13 | 0.73–0.88 | 0.46 | 0.25 |
| 50 (spin 200) | 243–251 k | **84–130 µs** | 1.2 ms | 189 µs | 0.13 | 0.74 | 0.48 | 0.26 |

Verdict (measured, not theorized):

1. **Hold ≈ 1.6 µs/commit** (wal 0.8 + mem 0.5 + publish 0.25 + prepare
   0.13; flush_check 0.02 when no flush fires). Serialization ceiling
   1/hold ≈ **640 k qps**.
2. **Writers block, not work**: avg wait 175 µs vs hold 1.6 µs (~99% of a
   writer's life is queued); measured 266 k = **2.4× below the ceiling** —
   the difference is park/unpark handoff under 50 threads / ~10 free CPUs
   (p99 1.2 ms = scheduling stalls).
3. **1 writer = 1.05 M qps; 50 writers = 266 k** — adding writers makes the
   engine 4× slower while still beating healthy Rocks (152 k) by ~2×.
4. **Spin-then-park falsified** (`PEDRA_WRITE_SPIN` 200: 243–251 k <
   266 k park, p50 6–10× worse — spinning burns the free CPUs). The
   convoy is not fixed by spinning.

### What this decides for the RFC

- **P1.1 (prepare off-lock) is falsified as the 5× lever**: prepare is
  0.13 µs of a 1.6 µs hold (8%). Moving it out buys ≤8%.
- The movable mass is **mem (31%)** and then wal (50%, the Rocks-shaped
  serialized part). Closing 5× (≈760 k in this window) requires hold ≤
  ~1.3 µs **and** cheaper handoff — i.e. P2.1 (memtable apply out of the
  critical section) plus a handoff that doesn't park every contender.
- `PEDRA_ASYNC_GROUP` merge remains rejected (0.19×, earlier A/B); its
  no-catch-up variant is the untested handoff shape (P2.2).

## P0.3 — deps_lock_prewrite: the 0.94 is suite-context, not the shape

Isolated paired rounds (fresh process, only this shape; async column
`PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`; load ~10–12, dirty):

| round | Pedra qps | Rocks qps | ratio |
|---|---:|---:|---:|
| 1 | 49 685 | 21 767 | **2.28** |
| 2 | 52 239 | 23 148 | **2.26** |
| 3 | 51 020 | 24 405 | **2.09** |

In-suite (P2.1 quiet arbiter, full v0 in one process): 1.40 / 0.94 / 0.77,
med 0.94. **The shape itself wins ≥2×; it loses only after the rest of v0
has run in the same process/db.** Mechanism (hypothesis for P1.2, not yet
profiled): accumulated state from earlier shapes (MVCC versions, lock-CF
residue, memtable/SST mix) — candidates are our `key_has_write_after` /
tail_ord cost growing with retained versions vs Rocks getting a warmer
memtable. P1.2 = bisect the suite context (which preceding shape flips
the sign), then profile the flipped case.

## Raw

- `/tmp/mwprobe-flushfree.txt` (copied below), spin sweep + 1c runs in
  session log; JSONs in `lockprewrite-r{1,2,3}/`.
- Anomaly note: instrumentation edits + probe were reverted from the
  working tree by an external actor at ~18:54 (re-applied, committed
  d5549d5). `.gitignore` modified by the same actor (bench DB ignores) —
  left in place, not ours to revert.

```
multiwriter_probe ops/thread=100000 records=4096 payload=1024B skew=squared-uniform flush=1GiB
threads=1  qps=416034 p50=0.75us p99=32.42us  lock_wait=0.00 prepare=0.08 wal=1.33 mem=0.39 publish=0.08 flush=0.023
threads=4  qps=378125 p50=1.00us p99=112.88us lock_wait=7.96 prepare=0.07 wal=0.95 mem=0.36 publish=0.10 flush=0.023
threads=12 qps=191054 p50=1.54us p99=303.67us lock_wait=53.50 prepare=0.09 wal=0.82 mem=0.40 publish=0.14 flush=2.339
threads=50 qps=137218 p50=7.83us p99=1162.04us lock_wait=344.19 prepare=0.12 wal=1.67 mem=0.44 publish=0.23 flush=2.901
(5M-op runs: flush amortization of the 1 GiB buffer is in-window — use the
50×2000 bench-shaped runs above for phase attribution)
```
