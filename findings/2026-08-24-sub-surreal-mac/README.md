# sub-surreal — SurrealDB v1.5.4 substitution, Mac (não-oficial)

Data: 2026-08-24 · Host: macOS aarch64 (load ~9, não quieto — números de
sanity, não oficiais) · Fonte: worktree rfc-0054-gaps @ d22048e

Substituição provada end-to-end: `surrealdb-core` v1.5.4 (tag upstream
`v1.5.4`, vendored em `bench/sub-surreal/surreal-core/`) **sem uma linha
alterada**, compilando contra o shim `crates/rocksdb` (package `rocksdb`
0.21.0) que reexporta `rocksdb-compat`. O peer é o mesmo driver contra
`rocksdb` 0.21 real (crates.io, C++ bundled). SurrealDB seta
`WriteOptions::set_sync(false)` no próprio caminho de transação — o peer
oficial de paridade.

## Resultados (SUB_BENCH_SECONDS=8, RECORDS=1024)

| leg | pedra qps | rocks qps | ratio |
|-----|-----------|-----------|-------|
| point_read (zipf, tx read) | 999.946 | 498.412 | **2,01×** |
| point_write (zipf, tx write) | 495.811 | 166.797 | **2,97×** |
| scan (kvs scan = raw_iterator+seek, rows/s) | 784.870 | 723.362 | **1,09×** |
| doc_txn (RMW get→set→commit, OCC) | 249.535 | 124.407 | **2,01×** |

Raw: `pedra.txt` / `rocks.txt` (linhas `SUB_BENCH_JSON`).

## Leitura

- O caminho transacional inteiro do SurrealDB (Mutex<Option<Transaction>>
  + snapshot + OCC + commit) sobre Pedra fica **2–3×** acima do RocksDB
  real no mesmo código upper — a vantagem do harness de paridade
  sobrevive à camada do banco de verdade.
- `scan` 1,09×: cada kvs `scan` reabre um `raw_iterator_opt` (re-seek por
  chamada) — o custo é dominado pelo open/seek do iterador, não pelo
  motor; forma aberta (ver RFC-0054 P1.3 depriorizado).
- `doc_txn` 2,01× confirma que a validação OCC (read-set no commit) não
  come a vantagem de escrita.

Linux/Intel oficial: `findings/2026-08-24-sub-surreal-linux/` (VM
`linux-sub-1`, imagem bench4sub).
