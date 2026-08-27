# Darwin coluna B — o smoke 2026-08-25 não é gate

**Fonte:** [`../2026-08-25-g1-vs-rocks-sync-ff/`](../2026-08-25-g1-vs-rocks-sync-ff/README.md)
JSON conferido 2026-08-27. Caixa agora: M2 Max, load 8.0/8.3/9.4, 18 users —
não dá 3/3 quieto.

## O que o 0.61 é

1 run, ops=200, load 15–23, binário `rfc-0054-gaps@3e3fdde` (pré
`lone_sync_commit` / intern / LAST_CF). Ambos `F_FULLFSYNC`
(`PEDRA_PARITY_G1=1`, `ROCKS_PARITY_FULL_SYNC=1`).

| shape | p50 P/R ms | p99 P/R ms | max P/R ms | qps ratio |
|---|---:|---:|---:|---:|
| **ycsb_a** | **4.851 / 4.731** | 28.2 / 9.0 | 64.9 / 9.1 | **0.611** |
| ycsb_d | 0.001 / 0.001 | 18.7 / 7.1 | **379 / 9.2** | 0.170 |
| apply | 59 / 15 | **2164 / 64** | **2946 / 66** | 0.168 |
| raftlog | 27.6 / 29.3 | 54 / 58 | 72 / 140 | **1.13** |
| ycsb_c | 0.0003 / 0.0005 | — | — | **1.69** |

ycsb_a **p50 empatou** (~4.8 ms = um `F_FULLFSYNC`). O 0.61× é qps puxado
pela cauda (p99 28 vs 9) numa caixa suja. ycsb_d p50 empatado/ganho; o
0.17× é um max 379 ms. Apply p99 2.2 s — contaminação, não cartaz.

Isto **não** prova “Mac + `set_sync(true)` = 0.61×”. Prova que, na mesma
classe `F_FULLFSYNC`, o syscall mediano empatou.

## O que o host Mac realmente chama

`librocksdb-sys` nesta caixa **não** tem `HAVE_FULLFSYNC`
(`engines.rs`: default Rocks `WriteOptions.sync` é `fdatasync` ~50 µs,
não `F_FULLFSYNC` ~5 ms).

Compat `Options::set_sync(true)` deixa `wal_full_fsync=true` (default).
Darwin G1 = `F_FULLFSYNC`.

| | Pedra `set_sync(true)` | rust-rocksdb `set_sync(true)` |
|---|---|---|
| barreira | `F_FULLFSYNC` ~5 ms | `fdatasync` ~50 µs |
| vs smoke 25/08 | o que se mediu (Rocks `FULL_SYNC=1`) | **não** foi o peer |

Host Mac que só faz `wopts.set_sync(true)` (API rust-rocksdb, sem knob
Pedra) sente **~100×** no 1c write, não 0.61×. É mais durável (vantagem)
e mais lento (defeito que o host sente). Linux coluna B não tem este
buraco: lá `sync=true` dos dois lados é `fdatasync` (`P11_PASS` 1.013).

O harness **não** tem env para `wal_full_fsync=false`. Não dá para medir
a classe rust-rocksdb no Darwin sem um corte no bench.

## O que falta para fechar Darwin B

1. Quiet 3/3, ops=2000, tree atual — **não** o smoke 200 ops / load 20.
2. Duas colunas, não uma:
   - **B-host:** Pedra `set_sync(true)` + `wal_full_fsync=false` vs Rocks
     `sync=true` `FULL_SYNC=0` (o que o host rust-rocksdb realmente paga).
   - **B-strong:** ambos `F_FULLFSYNC` (o smoke; p50 já empatou).
3. Sem (2) o default `wal_full_fsync=true` continua um defeito de
   velocidade no Mac para quem migra de rust-rocksdb com só `set_sync(true)`.
