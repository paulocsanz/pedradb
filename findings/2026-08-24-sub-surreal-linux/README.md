# Substituição SurrealDB no Linux — sub-5 (RFC-0059 P2.3)

VM `linux-sub-5`, 4 vCPU, 8 GB, AMD Threadripper PRO 3975WX.
Imagem `bench8sub` — fonte = tree ad2dbab (txn.rs F183 validado,
md5 confere com o commit; o tarball da sub-2 era pré-fix).
Entrypoint v3 (build com output streamado + timestamps).

SurrealDB v1.5.4, storage trocado: lado pedra usa o shim crate
`rocksdb` v0.21.0 (reexporta `rocksdb-compat`); lado peer usa
`rocksdb` real (librocksdb-sys 8.10). Mesmo binário `sub-bench`
(ops=2000, records=1024, 8s/leg), só `SUB_BENCH_ENGINE` difere.

Timeline: pedra build 12:29:42→12:33:23, peer build →12:42:14,
runs 12:42:14/12:43:01, `RESULT=SUB_BENCH_DONE 12:43:38Z`.

## Ratios (SUB_RATIO do serial)

| leg | pedra qps | rocks qps | ratio |
|---|---|---|---|
| point_write | 356 497 | 148 724 | **2.40×** |
| point_read | 649 058 | 359 003 | **1.81×** |
| doc_txn | 159 247 | 111 459 | **1.43×** |
| scan | 371 044 | 636 953 | **0.58×** |

Arquivos crus: `sub_surreal_linux.json`, `pedra.txt`, `rocks.txt`
(blob `BLOBSUB` decodificado do serial); extrato em
`serial-extract.txt`.

## Leitura

- Substituição provada no Linux: SurrealDB v1.5.4 roda inteiro sobre
  o shim (corretidade: kvs::tests 62/62 no Mac, F183 iterador de
  transação com overlay) e 3 das 4 pernas são mais rápidas que o
  RocksDB real — write 2.40×, read 1.81×, txn 1.43×.
- `scan` 0.58× é gap real do shim no padrão range-scan do SurrealDB
  (iterate + prefix). Não é o `kvrocks_scan` da bateria oficial
  (32.7× PASS): padrões diferentes — o scan do sub-bench varre o
  keyspace inteiro via raw iterator com seek por prefixo. Fica
  documentado como próximo item do shim (iterator com prefix-bloom /
  run-prefix), não bloqueia o claim de substituição.
