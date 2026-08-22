# P1.3 rearm4 — official-quiet attempt at 54d0b24 (NON-OFFICIAL, evidence)

Fourth official battery for the GET TLS-cache fix (312e354), now with all
12 RFC-0048 correctness fixes landed (54d0b24). Same hardened P0.4 method as
rearm3: 2-consecutive-sample gate (<10 load), watchdog armed only after the
gate passes, 3× (kvr + async n=2000) rounds + 2M ONLY-filtered long window,
peer = RocksDB default (`ROCKS_PARITY_SYNC=0`), median-of-3.

## Verdict

`NON-OFFICIAL` — the gate passed (7.69), but the watchdog caught load1=10.79
crossing the bar mid-battery (round 3). Evidence only, per P0.4 standard.

## Numbers (all ratios vs RocksDB default, sync=false)

| shape | med (3 rounds) | long 2M |
|---|---|---|
| kvrocks_get | **6.30** (6.00/6.30/6.66) 3/3 ≥5 | **6.49** |
| kvrocks_set | 4.54 (4.54/0.23/5.51) | 4.49 |
| kvrocks_pipelined_set | 5.08 (5.08/1.12/6.30) | 3.67 |
| kvrocks_scan | 34.43 | — |
| ycsb_a | 4.56 (7.74/4.56/2.14) | — |
| ycsb_f | 6.02 (6.02/3.18/7.71) | — |
| ycsb_c | 3.74 | — |

## Reading

- **GET is confirmed a third time**: 3/3 rounds ≥5 (median 6.30) plus the
  long window at 6.49 — same as rearm1 (6.28) and rearm3 (6.15). Three
  dirty batteries agree; the controlled composition (quiet P0.4 numbers)
  gives ≥6.1×.
- The wild per-round swings on write-heavy shapes (ycsb_a 7.74→2.14,
  set 5.51→0.23) are NOT peer noise: the ycsb_a profile taken during this
  window (see `../2026-08-22-writepath-fold-gc/`) shows the compat fold
  worker burning a full core in `MemTable::insert_map` memmove — the
  write-path quadratic fixed after this battery. The quiet-machine rounds
  (ycsb_a 7.74) show the headroom the fix should stabilize.
- Official closure still needs a quiet battery (this box never held <10 for
  a full window) or the caixote cloud VM (provisioning path dead on the
  current CLI/API version — see `/tmp/pedradb-iac` note in the rearm3
  findings).

Raw: `loads.txt` (gate + watchdog + per-leg loads), `r1..r3/`, `long/`.
