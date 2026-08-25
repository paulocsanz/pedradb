# Parity battery floor 1× — async same-class column (2026-08-24)

**This is NOT the official product column.** This run measured the RFC-0054
drop-in default: Pedra with `PEDRA_PARITY_ASYNC=1` (WAL write, no fdatasync
before Ok) vs RocksDB default (`WriteOptions.sync=false`, `ROCKS_PARITY_SYNC=0`).
Same durability class on both sides — engine speed only, never quoted as
"we beat Rocks". The official G1 column (Pedra fdatasyncs before Ok vs the
same Rocks default peer) is the sibling finding
[`rocks-parity-floor1x-g1/`](../rocks-parity-floor1x-g1/).

Context: this battery was launched before the G1 selector existed (RFC-0054
made async the drop-in default and left no way back to the durable column),
so the first numbers landed on the wrong column for a floor decision. Kept
here as the same-class reference; the floor flip decision uses the G1 run.

**This is the official gate column as of 2026-08-24** (RFC-0041 floor
re-baseline, registered product decision): `scripts/rocksdb_parity_v0.sh`
defaults `ROCKS_PARITY_RATIO_FLOOR=1.0` and measures this drop-in column —
the engine-speed regression gate (15/15 pass, min 1.254). Still never
quoted as "we beat Rocks"; the product claim stays in the G1 sibling.

Protocol: `scripts/rocksdb_parity_v0.sh` with
`ROCKS_PARITY_SYNC=0 ROCKS_PARITY_RATIO_FLOOR=1.0`, real RocksDB peer built
and run by `rocks_side_ycsb.sh` on the same box, single run, `compare_report.json`
produced by `rocks-parity-compare`. 15 shapes with peer data, all gated.

| shape | compat (async) | rocksdb (default) | ratio |
|---|---:|---:|---:|
| deps_lock_prewrite | 44212.7 | 12116.2 | 3.649 |
| ycsb_a | 2083333.3 | 599250.9 | 3.477 |
| ycsb_b | 5417705.1 | 1652319.0 | 3.279 |
| ycsb_f | 1930036.2 | 589318.6 | 3.275 |
| ycsb_b_unif | 5100869.7 | 1607497.4 | 3.173 |
| ycsb_c_unif | 6177606.2 | 2271643.1 | 2.719 |
| ycsb_c | 5693950.2 | 2160200.5 | 2.636 |
| deps_apply_batch | 22688.1 | 8786.0 | 2.582 |
| deps_cache_overwrite | 943209.4 | 383754.9 | 2.458 |
| ycsb_d | 4043835.2 | 1676558.4 | 2.412 |
| ycsb_e | 633328.3 | 270819.2 | 2.339 |
| ycsb_c_big | 349701.7 | 167369.8 | 2.089 |
| deps_mvcc_latest | 565770.9 | 393377.9 | 1.438 |
| deps_scan | 397548.7 | 316873.3 | 1.255 |
| deps_raftlog | 167791.1 | 133764.3 | 1.254 |

**min_ratio 1.254, pass (floor 1.0).** Single-node lab bench on this
machine; the honesty field in `compare/compare_report.json` states the
same-class caveat. Files: `compat/rocks_parity_bench.json` (Pedra side),
`rocks_side/rocks_shaped_peer.json` (Rocks side), `compare/compare_report.json`.
