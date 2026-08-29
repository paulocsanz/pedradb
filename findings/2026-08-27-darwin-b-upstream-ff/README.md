# Darwin coluna B vs **upstream** Rocks `F_FULLFSYNC`

**When:** 2026-08-27T02:52:33Z–02:55:01Z  
**Box:** M2 Max, load 7.2–10.7 / 18 users — **dirty**, not a 3/3 quiet gate.  
**Peer:** Pedra `PEDRA_PARITY_G1=1` (`F_FULLFSYNC`) vs Rocks `sync=true` **plus**
`ROCKS_PARITY_FULL_SYNC=1` (`File::sync_all` on `*.log`). That reconstructs
CMake Rocks `PosixWritableFile::Sync` (`HAVE_FULLFSYNC`). Not the
crates.io `librocksdb-sys` 0.16 `fdatasync` hole.

ops=2000 zipfian, tree atual (`lone_sync_commit`).

| shape | min | mediana | rounds (qps P/R) | p50 P/R ms |
|---|---:|---:|---|---|
| ycsb_a | **0.944** | 1.024 | 1.352 / **0.944** / 1.024 | **3.41 / 3.70**, 3.42 / 3.55, 3.61 / 3.52 |
| deps_raftlog | **0.944** | 1.009 | 1.009 / **0.944** / 1.469 | **4.02 / 4.00**, 4.07 / 4.01, 4.02 / 4.02 |

p50 **empatado** no `F_FULLFSYNC` (~3.5 ms A, ~4.0 ms raftlog batch).
min 0.944 é um round sujo (r2). Não arredondar a gate Darwin 3/3.
O 0.61 do smoke 25/08 (ops=200, load 20) era cauda, não o syscall.

`FULL_SYNC=1` reconstitui por cima: Rocks ainda faz `fdatasync` e o harness
`sync_all` **todo** `*.log`. Próximo: `CXXFLAGS=-DHAVE_FULLFSYNC` no
compile + `FULL_SYNC=0` (syscall no fd vivo, classe CMake). Caixa
2026-08-27 00:31 load 7–8 / 19 users — não é quiet 3/3.

JSON: `r{1,2,3}/{pedra,rocks}/rocks_parity_bench.json`.
Sources: [`../2026-08-27-upstream-fullfsync`](../2026-08-27-upstream-fullfsync/README.md).
