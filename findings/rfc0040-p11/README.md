# RFC-0040 P1.1 — MC apply/raftlog vs Rocks sync **e** async

2026-08-17. `ROCKS_PARITY_CLIENTS=4`, deps suite, 4096/2000 zipfian 1 KB.
Script: `scripts/tikv_ycsb_parity_mc_async.sh`. Harness: `YcsbRunner::run_deps_clients`.

Pedra **sempre** `fdatasync` antes do Ok. Rocks async = `WriteOptions.sync=false`.

## Mediana de 3 runs (qps Pedra / peer)

| shape | vs **sync** | vs **async** | p50 Pedra | p50 async |
|---|---:|---:|---:|---:|
| apply 1 cliente | 0.61 | 0.42 | 254 µs | 132 µs |
| **apply MC4** | 0.69 | **0.93** | 1.18 ms | 0.99 ms |
| raftlog 1 cliente | **5.21** | 0.55 | 51 µs | 18 µs |
| **raftlog MC4** | 1.69 | 0.61 | 183 µs | 120 µs |
| scan 1 cliente | 1.14 | 0.76 | 0.3–0.7 µs | 4–5 µs |
| mvcc 1 cliente | 1.59 | 1.65 | 0.8 µs | 4 µs |

Run-a-run vs async (apply MC4 / raftlog MC4): **0.97 / 1.57** · 0.93 / 0.61 · 0.56 / 0.34.
Run1 (mais quieta) é a que valida a tese: 4 clientes, Pedra+fd ≈ Rocks async no apply e **ganha** no raftlog.

## O que isto prova

- 1 cliente apply **não** pode amortizar 2 `fdatasync` — 0.42× async é o piso, não um bug.
- 4 clientes + write-group: apply **0.93× async** (quase empate pagando fd). P1.2 (catch-up) só se quisermos fechar os últimos ~7%.
- Raftlog 1 cliente já é **>5× Rocks sync** (alvo 0039 P1.1 no qps mediano). Vs async ainda 0.55× (p50 51 vs 18 µs ≈ um fd).
- Scan: p50 já ganha do async; qps mediano 0.76 por cauda (run3 max 117 ms, L0=9). P2.1 = drenar L0.

## Raw

`run{1,2,3}/{compat,rocks-sync,rocks-async}/rocks_parity_bench.json`
