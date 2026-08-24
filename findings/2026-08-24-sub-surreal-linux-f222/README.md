# Substituição SurrealDB no Linux pós-F222 — sub-6 (RFC-0059 P2.3)

VM `linux-sub-6`, 4 vCPU, 8 GB, AMD Threadripper PRO 3975WX.
Imagem `bench16sub` — fonte = tree `3c2c607` (F222: `TxnRawIterator`
sem clone por linha). Mesmo protocolo dos achados anteriores
(`findings/2026-08-24-sub-surreal-linux/` e `-f221/`): SurrealDB v1.5.4
inteiro, storage trocada só pelo patch, sub-bench ops=2000
records=1024 8s/leg, entrypoint v7 (offline, sem `/data`, run sem
target).

Timeline: build pedra 16:13→16:20, run pedra 16:20, build peer
16:21→16:29 (C++), run peer 16:29, `SUB_RATIO` 16:30:30Z.

## Ratios (SUB_RATIO do serial)

| leg | pedra qps | rocks qps | ratio | sub-5 pré | sub-6 F221 |
|---|---|---|---|---|---|
| point_write | 355 018 | 147 424 | **2.41×** | 2.40× | 2.64× |
| point_read | 627 873 | 350 308 | **1.79×** | 1.81× | 1.93× |
| scan | 797 266 | 598 810 | **1.33×** | **0.58×** | 1.10× |
| doc_txn | 143 989 | 103 154 | **1.40×** | 1.43× | 1.45× |

Arquivos crus: `sub_surreal_linux.json`, `pedra.txt`, `rocks.txt`
(blob `BLOBSUB` decodificado); extrato em `serial-extract.txt`.

## Leitura

- **F222 sobe o scan de 1.10× → 1.33×**: `TxnRawIterator::head()`
  alocava key+value (`Vec` novo) por linha do scan de transação;
  agora a cabeça é a origem (`Db` delega ao iterador db com posição
  estável; `Staged(idx)` referencia o overlay). Zero cópias por
  linha. O scan do SurrealDB roda dentro de transação de leitura, então
  essa é a perna que sente.
- write/read/txn ficam na banda de ruído entre VMs (peers variam ±6%
  entre runs: rocks write 132k/147k, read 345k/350k) — nada regrediu
  estruturalmente.
- Perfil Mac (medição direta db vs txn iterator): db/txn 1.11× →
  0.85–1.03× em 3 runs — overlay de transação a custo zero.
- Corretude: compat 49/49, SurrealDB `kvs::tests` 62/62 sobre o shim
  (inclui os testes de scan/transação que provaram o F183).
- Estado: **4/4 pernas >1×** (scan 1.33×, txn 1.40×, read 1.79×,
  write 2.41×) vs RocksDB real default `sync=false`.
