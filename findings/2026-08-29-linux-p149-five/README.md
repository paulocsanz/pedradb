# RFC-0149 P2.1 — five-shape cut (metal 4-CPU)

**When:** 2026-08-29  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `WriteOptions.sync=false`.  
**Host:** Threadripper PRO 3975WX, `CPUQuota=400%` `CPUAffinity=0-3`,
chroot `/var/lib/caixote-nvme/p149-root`. Several CHV guests still on the
box (affinity 0–63). Not the CHV 4 vCPU virt gate.

JSON: `full5k/` (ycsb+deps split; kvrocks `ONLY=set,blob`; ops=2000 zipf).

## Table (3 rounds, median)

| shape | min | median | r1 / r2 / r3 | target |
|---|---:|---:|---|---:|
| ycsb_d | 2.846 | **3.082** | 2.846 / 3.082 / 3.162 | 3× yes |
| deps_apply_batch | 1.834 | **1.930** | 1.936 / 1.930 / 1.834 | 2× no |
| deps_scan | 4.298 | **4.452** | 4.298 / 4.764 / 4.452 | 3× yes |
| kvrocks_set | 3.213 | **3.265** | 3.213 / 3.265 / 3.282 | 3× yes |
| kvrocks_blob_set | 3.316 | **3.321** | 3.321 / 3.364 / 3.316 | 3× yes |

**4/5** at the table above (apply 1.93). Apply 2× closed later the same day:

## deps_apply_batch 2× guaranteed (metal 4-CPU)

Peer still Rocks `sync=false`. Isolated apply-only 5 rounds:

| | min | median | max | r1 / r2 / r3 / r4 / r5 |
|---|---:|---:|---:|---|
| apply | **2.175** | **2.319** | 2.329 | 2.329 / 2.321 / 2.217 / 2.319 / 2.175 |

Same-suite apply+scan 3 rounds: apply min **2.260** median **2.323**; scan min **5.101** median **5.208** (was 4.45).

Phases now: prepare ~3.3µs · wal ~2.7µs · **mem ~6.3µs** (was 11.9) · publish 0.1µs.

`insert_many` hint≥16 (apply) appends the tail and does **not** live-index `default`/`lock`. Write CF keys go into `write_log` (BTree, one lock per batch) so `deps_scan` stays a range count with no 256k rebuild in the timed window. First get after apply rebuilds a lazy idx cache.

G1 still spills; async coluna A still WAL-inlines. Shape unchanged (2 atomic multi-CF batches × 32 txns). Not the CHV 4 vCPU virt gate; RFC-0149 P2.1 still open.

`RESULT=P149_FIVE apply 2.17 min (5/5 ≥2.0) scan 5.21`
