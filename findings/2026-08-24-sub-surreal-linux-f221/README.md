# Substituição SurrealDB no Linux pós-F221 — sub-6 (RFC-0059 P2.3)

VM `linux-sub-6`, 4 vCPU, 8 GB, AMD Threadripper PRO 3975WX.
Imagem `bench15sub` — fonte = tree `e9fcf29` (F221 `04dce83`:
`MemChunkStream`, fim da materialização quadrática do memtable por
janela de iterador). Mesmo protocolo do sub-5 (`findings/2026-08-24-sub-surreal-linux/`):
SurrealDB v1.5.4 inteiro, storage trocada só pelo patch (`rocksdb` 0.21
shim vs librocksdb-sys real), sub-bench ops=2000 records=1024 8s/leg.

Timeline: build pedra 15:28→15:32, run pedra 15:32, build peer
15:33→15:42 (C++), run peer 15:42, `SUB_RATIO` 15:43:19Z.

## Ratios (SUB_RATIO do serial)

| leg | pedra qps | rocks qps | ratio | antes (sub-5, pré-F221) |
|---|---|---|---|---|
| point_write | 350 534 | 132 933 | **2.64×** | 2.40× |
| point_read | 665 271 | 345 391 | **1.93×** | 1.81× |
| scan | 708 896 | 641 831 | **1.10×** | **0.58×** |
| doc_txn | 150 139 | 103 593 | **1.45×** | 1.43× |

Arquivos crus: `sub_surreal_linux.json`, `pedra.txt`, `rocks.txt`
(blob `BLOBSUB` decodificado); extrato em `serial-extract.txt`.

## Leitura

- **F221 fecha o gap de scan**: 0.58× → 1.10× no mesmo padrão range-scan
  do SurrealDB (raw iterator + seek por chamada). Causa raiz era no core,
  não no shim: `memtable_stream` materializava o range `[start,end)`
  inteiro a cada refill de janela de 64 linhas (quadrático sobre scans
  longos; amostrado no Mac como realloc+memmove no hot path). O
  `MemChunkStream` refaz o iterador por chunk de 256 retomando de
  `Excluded(last)`. Perfil Mac (mesma máquina, antes→depois): scan0
  1.62M→3.79M rows/s, scan1 versionado 1.23M→2.77M.
- As outras três pernas também subiram (write 2.40→2.64, read
  1.81→1.93, txn 1.43→1.45) — o F221 tira trabalho redundante do
  caminho de leitura em geral, não só do scan.
- Lado rocks do sub-6 bate com o sub-5 (±4%): peer comparável entre
  VMs.
- Substituição no Linux agora **4/4 pernas >1×** o RocksDB real
  (default `sync=false`) com o upper DB intacto.

## Incidentes de infra resolvidos no caminho (registro)

- **DNS da região caiu** ~13:44Z (index.crates.io sem resolver por
  >1h): builds passaram a ser `--offline` com vendor mesclado
  (`bench/sub-surreal/vendor.sh`, commit `e9fcf29`; imagens bench12+).
- **Volume de dados `/dev/vdb` com EIO** (kernel guest: "EXT4-fs (vdb):
  failed to convert unwritten extents"): entrypoint v6+ não toca mais
  em `/data`; targets no overlay com limpeza entre pontas.
- **ENOSPC no run pedra** (writable ~2 GB tmpfs dividido com o target
  do build): entrypoint v7 copia o binário, deleta o target e só então
  roda (`bench15sub`).
