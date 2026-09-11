# RFC-0190 P0.4 / P1.1 / P1.2 / P2.1 / P2.2 — Darwin DIAG + Linux blocked

**Date:** 2026-09-10
**Host:** Darwin, loadavg 7.48 / 9.71 / 11.28 at WRITEPHASE (not quiet).
**Linux guest:** `linux-gate-p149b` — hostname does not resolve (`UNREACHABLE`).
**Peer JSON:** `sync: false` on every compare that ran (refuse-if-sync-peer held).
**Not cartaz. Not a KEEP. Not a win.**

## P0.4 serial (this is the one number that is not DIAG)

`cargo test -p pedradb-core --lib -- --test-threads=1`

| | passed | failed | ignored |
|---|---|---|---|
| baseline (pre P0.3) | 874 | 22 | 4 |
| after P0.3 lanes | 880 | 21 | 4 |

Zero new failures. +6 passes = 4 named `rfc0190_*` tests + 1 lane test +
`adaptive_spin_absorbs_slow_leader` (baseline flake, not claimed). Recovery
`reopen_recovers_puts` + `torn_tail_*` green.

## Darwin DIAG ratios (`ROCKS_PARITY_SYNC=0`, one run, noisy box)

| cell | compat ops/s | rocks ops/s | compat_over_rocksdb |
|---|---|---|---|
| deps_cache_overwrite_mc4 | 169792.8 | 175819.3 | **0.966** |
| deps_apply_batch_mc4 | 10574.5 | 21653.0 | **0.488** |
| kvrocks_set_mc50 | — | — | **not finished** (>12 min hung on 50-client Darwin DIAG; killed) |

Scratch: `meter-<cell>.log` + `meter-<cell>/compare/compare_report.json`.

WRITEPHASE `deps_cache_overwrite_mc4` after lanes (same noisy box):
`guard=0.05µs mlock=0.03µs mins=0.80µs mem=1.25µs lock_wait=6.66µs`
(baseline after P0.2: guard=0.09 mlock=0.04 mins=0.49 mem=0.88 lock_wait=10.27).

## P1.1 non-condition

TCB crate `pedradb-memtable` is gated on **cartaz** P0.4 < 1.0 **and**
WRITEPHASE naming `mlock`/`mins`. Linux cartaz did not run. Darwin DIAG
names `mins=0.80µs` as the largest mem slice — that is a hypothesis for
the next quiet Linux fire, not a trigger. Core still
`#![forbid(unsafe_code)]`. No `SAFETY.md` added.

## P1.2 C / blocked

`kvrocks_set_mc50` Linux 3-run unreachable. Darwin DIAG 50-client run did
not complete. Verdict **C** (cannot claim ≥1.0; cannot name a mechanism
ceiling from this host).

## P2.1 non-condition

Cross-shard idx composition is gated on P1.1 landing. P1.1 did not land.
8-lane tables already take the owned-sort merge (correctness floor);
the 1-lane idx cursor is unchanged.

## P2.2 blocked

Grid B (10–100× dataset, compaction on) is a Linux cartaz perna. Guest
unreachable. No run.

## Re-open

On a quiet Linux p149b 3-run: P0.4 overwrite_mc4 min-of-3, apply_mc4,
kvrocks_set_mc50; if overwrite < 1.0 and WRITEPHASE still names `mins`,
P1.1 fires and the skiplist TCB is the next cut — difficulty is not a
ceiling.
