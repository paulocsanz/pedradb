# 2026-08-30 — async class fix, CHV 17-shape re-measure (flush-per-commit)

**Change under test:** `88e2a63` — async commits `write()` the WAL frame
per commit (RocksDB default process-crash class); `ASYNC_WAL_BUFFER`
64 KiB userspace staging deleted. User decision 2026-08-30: fix the
guarantee, not the disclaimer.

**Peer:** real RocksDB default (`WriteOptions.sync=false`,
`ROCKS_PARITY_SYNC=0`), same run, same guest. 3 rounds, 17 shapes.
Guest `linux-gate-p149b`, fresh overlay from backing image, full
`pedradb-core/src` + `rocksdb-compat/src` + parity bench injected
(verified: 0 staging refs, forced-write sites present).

## Result

`RESULT=P149_FAIL over_med=8/17 min_ratio=0.623` (18:02:07Z)

| shape | staging median (P1.8 hold) | flush-per-commit median |
|---|---:|---:|
| ycsb_a | 3.42 | 1.81 |
| ycsb_b | 3.52 | 2.65 |
| ycsb_c | 3.53 | 3.78 |
| ycsb_d | 3.66 | 3.10 |
| ycsb_e | 14.7 | 14.4 |
| ycsb_f | 2.80 | 1.97 |
| deps_cache_overwrite | 2.48 | 1.64 |
| deps_lock_prewrite | 2.41 | 2.13 |
| deps_mvcc_latest | 4.02 | 4.33 |
| deps_apply_batch | 2.31 | 2.03 |
| deps_raftlog | 1.02 | **0.94** (rounds 0.968/0.942/0.623) |
| deps_scan | 5.11 | 4.34 |
| kvrocks_get | 4.77 | 4.71 |
| kvrocks_set | 2.91 | 1.62 |
| kvrocks_scan | 55.1 | 56.2 |
| kvrocks_pipelined_set | 4.17 | 3.42 |
| kvrocks_blob_set | 2.49 | 2.26 |

## Reading

- Reads unchanged (ycsb_c/e, kvrocks_get/scan) — WAL is not in the read
  path; those wins are engine.
- Write shapes lost 1.5–2×: the 64 KiB staging was a material share of
  the old async write ratios (matches the local A/B, ~1.75× on
  single-client write-per-op).
- **Floor breach:** `deps_raftlog` median 0.94 (< 1.0) breaks the
  RFC-0041 registered floor on the same-class drop-in column; majority
  ≥3× falls 12/17 → 8/17 (entrypoint gate needs ≥9 → FAIL).
- **G1 column untouched:** this change does not touch the product path
  (write + fdatasync before Ok); `findings/rocks-parity-floor1x-g1/`
  stands.

## Disposition

Kept, not refused: the change is a durability-class fix explicitly
ordered by the user after the local A/B tradeoff was presented. The
async column's definition changed (per-commit `write()`), so the
registered RFC-0041 floor and the P1.8 hold need a decision:
re-baseline the floor, recover raftlog with a perf slice, or revert.

Files: `serial.log` (full boot log, build 17:53:50Z → result 18:02:07Z),
`gate.txt` (final table).
