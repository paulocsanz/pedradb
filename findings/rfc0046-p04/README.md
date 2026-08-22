# RFC-0046 P0.4 quiet arbiter — v1 execution DISCARDED, re-armed (v2)

## What happened

The v1 arbiter (armed 2026-08-21 evening, fired at 01:50:44 when the
1-min load dipped to 8.91) ran to completion, but its official rounds
are invalid and the long window is marginally contaminated. All of it
is discarded; v2 (same script, fixed) is re-armed and waits for the
next quiet window.

## Why discarded

1. **Script bug — official rounds measured at smoke size.** The bench
   binary defaults to `ROCKS_YCSB_OPS=200` (a smoke default), and the
   script only exported `ROCKS_YCSB_OPS` for the 2M long window. The
   three official rounds (g1 / async / kvr) therefore ran with n=200
   per shape: individual shapes completed in milliseconds
   (`wall_s` 0.0027 for ycsb_a), making the qps ratios pure noise —
   see `r1/g1/compat/rocks_parity_bench.json` (`"ops=200"` in notes).
   The established p34 quiet method is n=2000
   (`findings/rfc0044-p2/quiet/run1/*/rocks_parity_bench.json`).
   Pairing itself was verified correct (peer `sync: false` in every
   round — RocksDB default, the official peer; 12 shapes paired for
   g1/async, 6 for kvr), so the fixed re-run is methodologically sound.
2. **Long window contamination.** The long compat leg (01:51:28 →
   02:08:02) overlapped a concurrent P2.8 development run
   (single-threaded example, ~70 s of the ~17 min leg; load held
   11.8–12.5) — the Pedra side of the long ratios is conservatively
   depressed by roughly that share. The long rocks leg overlapped the
   tail of that run by seconds (<1%).

For the record only (not usable as evidence): the contaminated long
window landed in the known p34 straddle band — `kvrocks_get` 4.58,
`kvrocks_set` 5.40, `kvrocks_pipelined_set` 4.62. The full v1 output
(rounds at n=200) is in `loads.txt` and `r1..r3/`, `long/`; v2
overwrites these directories when it fires.

## The fix

`scripts/rfc0046_p04_quiet_arbiter.sh` now exports
`ROCKS_YCSB_OPS=2000` for the three official rounds (p34 method) and
unsets it before the long window, which keeps its explicit 2M. Gate
unchanged (1-min load < 10, polls every 60 s, auto-fires).
