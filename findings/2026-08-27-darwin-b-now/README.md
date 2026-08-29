# Darwin coluna B — live WAL, ran now (no quiet wait)

`start 2026-08-27T07:14:20Z  4:14  up 9 days,  2:19, 17 users, load averages: 8,88 9,87 9,58`
`=== round 1 07:14:20 { 8,88 9,87 9,58 } users=17 ===`
`=== round 2 07:14:53 { 11,36 10,37 9,77 } users=17 ===`
`=== round 3 07:15:31 { 10,54 10,28 9,76 } users=17 ===`

| shape | min | mediana | rounds qps P/R | p50 P/R ms |
|---|---:|---:|---|---|
| ycsb_a | **0.909** | 0.999 | 1.007 / 0.999 / 0.909 | 3.43 / 3.53 |
| deps_raftlog | **0.999** | 1.003 | 1.003 / 1.003 / 0.999 | 4.01 / 4.00 |

Load 9–11 / 17 users. **Not** a quiet 3/3 gate.

p50 empatado no `F_FULLFSYNC` (A ~3.5 ms, raftlog 4.01/4.00). raftlog min 0.999 é ruído.
ycsb_a min 0.909 = r3 Pedra p99 8.1 vs Rocks 4.3 (cauda). Peer = live `NNNNNN.log` only.

JSON: `r{1,2,3}/{pedra,rocks}/rocks_parity_bench.json`.
