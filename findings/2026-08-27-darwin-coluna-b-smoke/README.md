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

## O que o host Mac realmente chama (CMake, não o sys-crate)

`librocksdb-sys` 0.16 desta árvore **omite** `HAVE_FULLFSYNC` — buraco
do binding, não do Rocks. CMake / Makefile do C++ Facebook **detectam**
`F_FULLFSYNC` e o `Sync()` do WAL é `fcntl(F_FULLFSYNC)`. Compat OOTB
`wal_full_fsync=true` casa **isso**, não o `fdatasync` do crate 0.22.

Comparar Pedra `set_sync(true)` contra rust-rocksdb 0.22 `fdatasync`
(~100×) **não** é coluna B. É Pedra CMake-class vs binding aleijado.
Não é defeito do substituto.

Linux coluna B: ambos `fdatasync` (`P11_PASS` 1.013). Darwin coluna B:
ambos `F_FULLFSYNC`.

## O que falta para fechar Darwin B

1. Quiet 3/3, ops=2000, tree atual — **não** o smoke 200 ops / load 20.
2. Peer = CMake class: `CXXFLAGS=-DHAVE_FULLFSYNC` no compile do
   `librocksdb-sys`, `ROCKS_PARITY_FULL_SYNC=0`. O `FULL_SYNC=1`
   (File::sync_all em todo `*.log`) é reconstituição; extra `fdatasync`
   + pode syncar WAL reciclado. Dirty 2026-08-27 já empatou p50 com
   isso (`../2026-08-27-darwin-b-upstream-ff`, min 0.944).
3. Não abrir coluna “B-host vs crate fdatasync”. Isso relitiga o
   binding hole.
