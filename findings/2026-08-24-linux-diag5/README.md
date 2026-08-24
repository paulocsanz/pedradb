# Linux diag-5 — deps_raftlog / deps_apply_batch isolados (RFC-0059 P0.3)

VM `linux-diag-5`, 4 vCPU, 8 GB, AMD Ryzen Threadripper PRO 3975WX
(host vocêki, kernel 6.12.94-0-virt). Imagem `bench9diag`
(fonte 00:40 = era d22048e, idêntica à bateria anti-1; entrypoint v3
com `PEDRA_WRITE_PHASE_STATS=1`).

5 rounds pedra (`PEDRA_PARITY_ASYNC=1`, `ROCKS_YCSB_OPS=4000`) vs
5 rounds rocks (`ROCKS_PARITY_SYNC=0` = peer oficial RocksDB default),
mais 2 rounds `ROCKS_DEPS_FOLD_TAIL=1`.

Boot 12:10:18Z, `RESULT=BUILD_OK` ~12:14Z, ratios 12:22Z.
Extrato completo do serial: `serial-extract.txt`.

## Ratios

| shape | r1 | r2 | r3 | r4 | r5 | mediana |
|---|---|---|---|---|---|---|
| deps_raftlog | 0.738 | 0.768 | 0.790 | 0.814 | 0.780 | **0.780** |
| deps_apply_batch | 1.729 | 2.118 | 2.045 | 1.869 | 1.954 | **1.954** |

`deps_raftlog` isolado confirma a bateria anti-1 (0.785/0.910/1.465,
mediana 0.910): a regressão Linux é real e estável, ~0.78×.
`deps_apply_batch` fica colado no piso (2/5 rounds ≥ 2.0; mediana 1.95).

## Fold-tail (H1: tail do memtable em keys monotônicas)

| shape | r1 | r2 |
|---|---|---|
| deps_raftlog | 0.947 | 1.029 |
| deps_apply_batch | 1.234 | 0.999 |

Fold pré-loop não recupera throughput → **H1 eliminada**.

## Fases por commit (`PEDRA_WRITE_PHASE_STATS=1`)

deps_raftlog (n=4000/round, batch de 16 ops):

```
prepare=0.60–1.15µs  wal=3.2–5.1µs  mem=3.5–4.3µs
publish=1.7–2.0µs    flsh=0.03µs    lock_wait=0.00µs
```

deps_apply_batch (n=8000/round, 2 commits/op):

```
prepare=2.7–3.7µs  wal=6.9–9.8µs  mem=12.4–16.2µs
publish=0.10–0.37µs  flsh=0.03µs  lock_wait=0.00µs
```

→ **H2 (wal/fsync) e H3 (publish/lock) eliminadas**: wal é só append
(coluna async, sem fdatasync), lock_wait é zero em ambos os shapes.

## Atribuição: cauda extrema fora do caminho de escrita

deps_raftlog por op (round de 4000 ops):

- p50 build do batch: **1.4µs** (`split p50 build=1.32–3.10µs`)
- p50 chamada `batch()` inteira: **9.7µs** (`split p50 batch=8.6–10.7µs`)
- soma das fases (média): ~11µs/commit
- wall médio por op: **~240µs** (64000 appends / ~0.96s, 4000 ops)

Caminho de escrita todo medido ≈ 11µs; média wall 240µs →
**~95% do tempo está em stalls fora do commit** — p50 ~10µs com
poucos ops demorando ~100ms+. `mem_after=1638676` constante entre
rounds (tail fold mantém, memtable não encolhe).

Hipótese remanescente (H4): trabalho de flush/compactação da memtable
em background disputando as 4 vCPUs — no Mac (mais cores) o mesmo shape
passa (>1.0); na VM de 4 vCPUs o flush de um run de 1.6M entries
periodicamente mata o writer. Alternativa: comportamento do allocator
no padrão append-heavy. Discriminador próximo: histograma de latência
por op com atribuição de stall (logar ops >1ms com thread), ou fixar
o flush numa core dedicada e re-medir.

deps_apply_batch: caminho de commit domina (mem 12–16µs, wal 7–10µs,
build p50 15–16µs); sem cauda anômala — é custo real de memtable/wal,
não stall. Está no piso (1.95 mediana), não abaixo dele por patologia.

## Nota de blob

`DIAG_BLOB_SIZE=145296031` — o tgz dos JSONs crus (com arrays de
latência por-op) é grande demais para o canal serial (replay duplicaria
~194MB de base64 a cada recycle). Ratios, fases e splits acima são o
extrato completo do serial; os JSONs crus não foram arquivados.

## Conclusão P0.3

- `deps_raftlog` Linux = gap real **documentado e atribuído**
  (cauda fora do caminho de escrita; H1/H2/H3 eliminadas; H4
  flush-bg/4vCPU é a hipótese viva). Sem fix: um fix direcionado ao
  padrão do benchmark sem ganho geral seria overindexing — o próximo
  passo é o discriminador de stall, não um tweak.
- `deps_apply_batch` Linux = colado no piso (mediana 1.95, Mac
  2.07–2.18). Custo real de commit, não patologia.
- Bateria oficial (13/16 ≥2×) permanece como está em
  `findings/2026-08-24-linux-anti1/`.
