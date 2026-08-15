# Montanha vs FoundationDB — FDB-shaped microbenchmarks

**Status:** lab method (v0)  
**Binary:** `montanha-fdb-bench`  
**Not:** YCSB field peer · Apple Simulation · production SLA claim

## Why this exists

Functional phases (1–3, A–E) prove **correctness seeds**. Benches prove **where Montanha is expensive** relative to the FDB mental model:

| Cost center | FDB (typical) | Montanha (today) |
|-------------|----------------|------------------|
| Single key durable write | proxy → storage → disc | multi-Raft majority + Pedra WAL/LSM |
| Multi-key TX same shard | optimistic commit | snapshot TX + OCC on one range leader |
| Multi-key cross shard | GRV + resolve + commit | **2PC across ranges** (expect cliff) |
| Range read + conflict | ordered keyspace | `get_range` + conflict ranges |
| Hot key | conflict + retry | same, but serial client in this harness |

The goal is to **find cliffs**, not to win a marketing table.

## Run (Montanha)

```bash
# Release matters — debug numbers are not comparable.
# Suites: core | threads | tcp | mini-bt | all  (default: core,threads,mini-bt,tcp)
cargo run -p pedradb-store --release --bin montanha-fdb-bench -- findings/fdb-bench-local
cargo build -p pedradb-store --release --bin montanha-tcp   # required for suite=tcp

# Larger / heavier
MONTANHA_BENCH_SUITE=all MONTANHA_BENCH_N=100 MONTANHA_BENCH_THREADS=8 \
  MONTANHA_BENCH_PAYLOAD=256 MONTANHA_BENCH_TX_KEYS=16 MONTANHA_BENCH_RANGES=8 \
  cargo run -p pedradb-store --release --bin montanha-fdb-bench -- findings/fdb-bench-heavy

# TCP + mini-bt only (faster loop on network path)
MONTANHA_BENCH_SUITE=tcp,mini-bt MONTANHA_BENCH_N=40 \
  cargo run -p pedradb-store --release --bin montanha-fdb-bench -- findings/fdb-bench-tcp
```

Output: `findings/.../fdb_shaped_bench.json`

Also keep the simpler RFC-0021 gate:

```bash
cargo run -p pedradb-store --release --bin montanha-perf-gate -- findings/perf-gate-local
```

### Workloads in `montanha-fdb-bench`

| ID | Shape (FDB analogy) | What it stresses |
|----|---------------------|------------------|
| **A1** raw put | `set` durable | raft+Pedra write path |
| **A2** raw get | `get` | read / LocalApplied |
| **A3** fdb face set+commit | binding `set`+`commit` | face + OCC vs raw put |
| **A4** multi-key snapshot TX | multi-key transaction | same-range multi-key |
| **A5** get_range | `getRange` | range scan + conflict bookkeep |
| **A6** clear_range | `clearRange` | multi-clear staging + commit |
| **A7** hot-key WW | two TXs one key | conflict ratio / abort path |
| **A8** row+index TX | Record-layer-ish | 2-key maintain index |
| **B1** disjoint multi-range put | multi-proxy / multi-shard | fan-out by range |
| **B2** cross-range TX | multi-shard TX | **2PC tax** (main cliff candidate) |
| **D1–D2** TCP put/get | real localhost TCP majority | network path vs A1/A2 |
| **D3** TCP CommitTx | multi-key over wire | TX + TCP |
| **D4** TCP multi-thread put | N client threads | **true concurrent clients** |
| **D5** TCP DCS create/get | etcd-need over TCP | exclusive create + majority |
| **D6** TCP PutBatch | multi-key one RTT | put_many over wire |
| **S1** scale ranges sequential | disjoint put @ 1/2/4/8 ranges | single client (flat) |
| **S2** scale multi-client multi-range | N TCP threads, key→range | **option A proof** |
| **S3** multi-client multi-range PutBatch | N TCP threads × batch | batch amortize + multi-leader |
| *(client)* per-range leader cache | `active_range` + `warm_leaders` | multi-Raft dial without wrong prefer |
| **E1** mini-bindingtester | random set/clear/get/range + multi-key + WW | silent-wrong soak |
| **E2** TCP multi-client mini-bt | N threads, partitioned keys, majority verify | concurrent writers |

## How to compare to FDB (same shapes)

Montanha numbers alone are not “vs FDB”. Run **matching shapes** on an FDB lab cluster and fill the table.

### Suggested FDB side (sketch)

1. Local `fdbserver` single process or 3 storage + 1 proxy (document topology).  
2. Use `fdbcli` / bindingtester-style Python / C binding with **sync commit** (not fire-and-forget).  
3. Match: payload size, keys per TX, range size, single-threaded client first.  
4. Record: ops/s, p50/p99 latency, machine, disk, `knob` / version.

Example measurement targets:

| Workload | FDB API | Montanha bench ID |
|----------|---------|-------------------|
| Point write | `tr[k]=v; tr.commit().wait()` | A3 (face) or A1 (raw) |
| Multi-key | N sets + commit | A4 |
| Range | `tr.get_range(begin, end)` | A5 |
| Clear range | `tr.clear_range` | A6 |
| Conflict | two clients one key | A7 |
| Cross-shard | keys in distant subspaces | B2 (FDB shards ≠ Montanha ranges — label!) |

### Comparison rules (honesty)

1. **Label durability:** fsync / `sync=true` / FDB storage durability class.  
2. **Label topology:** in-process 3-node Montanha ≠ 3-machine FDB.  
3. **Label client threads:** this harness is **1 client thread**.  
4. **Never** compare debug Montanha to release FDB.  
5. Cross-range (B2) is **not** the same as FDB multi-shard without measuring both.  
6. Report **ratios**, not absolute winners: `A3/A1`, `B2/A4`, `A5 vs A2`.

### Ratio template (fill after both runs)

| Ratio | Meaning | Red flag if… |
|-------|---------|----------------|
| A3 / A1 | Face+OCC overhead | ≫ 2–3× unexplained |
| A4 latency vs A1×keys | Multi-key amortization | Superlinear |
| B2 / A4 | Cross-range 2PC tax | Large cliff (expected; quantify) |
| A5 vs A2 | Range vs point | Range path pathological |
| A7 abort_ratio | Conflict detection | ≠ ~0.5 commits aborting per pair |

## Known Montanha limitations these benches should surface

1. **Cross-range 2PC** — B2 slower than single-range multi-key; layers should co-locate hot keys.  
2. **Serial client** — aggregate cluster QPS higher with N clients / N range leaders (B1 starts that story).  
3. **In-process majority** — understates real TCP; pair with `tcp_multihost` / `montanha-tcp` soaks for network path.  
4. **OCC abort path** — A7 must stay correct; perf of retries is app-level.  
5. **Range + hist / SI** — large `get_range` may pay history/scan costs not in FDB Redwood.  
6. **No GRV service** — read version is cluster generation; different scaling story than FDB proxies.

## What would make this “FDB-class” as a method (not a number)

- Generated load (mini-bindingtester) mixed with latency histograms  
- Multi-threaded clients + open-loop vs closed-loop  
- Fault during bench (kill leader mid-run) — latency under recovery  
- Side-by-side JSON from FDB binding in CI when `FDB_CLUSTER_FILE` is set  
- TCP/multiproc variants of A3/A4/B2

## Related

- [montanha-fdb-phases.md](montanha-fdb-phases.md) — functional seeds  
- [montanha-vs-foundationdb.md](montanha-vs-foundationdb.md) — architecture comparison  
- RFC-0021 perf gate v0 — `montanha-perf-gate`  
- `findings/perf-*` — historical gate artifacts  

## Sample local run (laptop, release, N=25)

Machine-local only — **not** FDB comparison numbers. Use for ratios.

| Bench | qps | p50 ms | p99 ms |
|-------|-----|--------|--------|
| A1_raw_put_1range | 2.25 | 445.0 | 515.1 |
| A2_raw_get_1range | 820802.42 | 0.0 | 0.0 |
| A3_fdb_face_set_commit_1k | 0.96 | 1009.6 | 1700.2 |
| A4_snapshot_tx_4keys_1range | 0.64 | 1317.9 | 4605.4 |
| A5_get_range_prefix | 30670.14 | 0.0 | 0.1 |
| A6_clear_range_4keys | 0.87 | 1110.9 | 1445.7 |
| A7_hot_key_ww_conflict | abort_ratio=0.5 | success_p50=1075.0 | — |
| A8_record_row_plus_index_tx | 1.00 | 949.0 | 1541.6 |
| B1_disjoint_put_4ranges | 2.14 | 439.1 | 663.0 |
| B2_cross_range_tx_4keys | 0.38 | 2661.7 | 2960.0 |

### Observed ratios (this run)

- **A1/A3 face tax**: raw put is **2.33×** face commit QPS (face+OCC overhead)
- **A4/B2 cross-range tax**: same-range multi-key is **1.71×** cross-range 2PC QPS
- **A1/A4 multi-key**: point put is **3.50×** 4-key TX QPS

**Cliff:** B2 p50 ≈ 2.6s on this laptop — layers must co-locate keys when possible.

## Sample TCP + mini-bt (v1, N=16, 4 threads)

| Bench | Result |
|-------|--------|
| D1_tcp_put | ~3 qps, p50 ~330 ms |
| D2_tcp_get | ~6k qps, p50 ~0.16 ms |
| D3_tcp_commit_tx_2k | 15/15 ok, ~2.5 qps |
| D4_tcp_mt_put_4thr | 16 ok, aggregate ~3.3 qps (p50 higher under contention) |
| D5_tcp_dcs | rev=1, exclusive_fail=true, majority_seen=3 |
| E1_mini_bt | **pass**, mismatches=0, ~1.9 ops/s model-checked |

**Nuance:** multi-thread TCP does **not** linear-scale put QPS on a single range leader — expect p50 inflation (leader serializes). Aggregate QPS can still match or beat single-thread D1 when dial/retry is healthy.

## Sample E1/E2 correctness (N=24, 4 thr)

| Check | Result |
|-------|--------|
| E1 multi_key_ok | 23 |
| E1 ww_pairs_ok | 23 (exactly-one-winner) |
| E1 mismatches | **0** |
| E2 ops_ok / ops_err | 20 / 0 |
| E2 verified_majority | **29/29** |
| E2 mismatches | **0** |

```bash
MONTANHA_BENCH_SUITE=mini-bt,tcp MONTANHA_BENCH_N=40 MONTANHA_BENCH_THREADS=4 \
  cargo run -p pedradb-store --release --bin montanha-fdb-bench -- findings/fdb-bench-e2
```

## CI gates (not optional benches)

```bash
# Always-on model soak + WW/multi-key
cargo test -p pedradb-store --test mini_bt_soak mini_bt_inprocess -- --nocapture

# Concurrent TCP writers + majority verify
cargo test -p pedradb-store --test mini_bt_soak mini_bt_tcp_multiclient -- --nocapture
```

These fail the build on silent-wrong; the bench binary remains for latency/QPS cliffs.
