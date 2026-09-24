# RFC-0257 — EG1 A7 (`deps_scan` 100M @4GiB): scan fora do cache limitado

**Estado:** P0 DIAG em andamento (`:p259d` → `:p261d`)
**Paga:** EG1 A7 — `deps_scan` 100M registros, razão ≥ 1,0 vs RocksDB
default `sync=false` (a célula do cartaz hoje modela 0,70×)

## O gargalo (pré-DIAG, arqueologia)

`deps_scan` = varredura curta no CF write:
`scan_count_cf("write", ukey(u), ukey(u+25), 25)`, zipf, payload 100B,
1 cliente, records=100M. O RFC-0195 readahead é inerte aqui por
construção: valores 1B → janela de 25 chaves = 1 bloco → `run < 2`.

Donos candidatos:

- **(a) cache miss frio por op** — cada op paga I/O que o Rocks acerta.
  Contra-evidência: o caminho de contagem já é cacheado
  (TLS `LastCountTable` LAST_N=4096 + `CountCache::new(8192)`
  compartilhado — `count_named` em `rocksdb-compat/lib_kernel.rs:3244`,
  hit em `count_in_range` `db_kernel.rs:4178`); janelas zipf repetem e
  acertam. Blocos: `BlockCache` simétrico em entradas com Rocks (8192).
- **(b) custo fixo por op (setup/CPU)** — construir o iterador/merge por
  varredura. A favor: seções `scan_sst_setup_ns`/`scan_merge_ns` do probe
  vs `blocks_decoded`/`block_cache_misses` arbitraram (a) vs (b).

## P0 — DIAG `:p259d`/`:p261d`

`deps_scan` 25M e 100M, compat+rocks, probe split. **Achado :p259d:**
o braço compat 25M foi **OOM-killed** (exit 137, anon-rss 3,6 GiB num box
de 3,9 GiB) ~5 min após o início — stderr descartado, sem atribuição de
fase. Contraste: `deps_cache_overwrite`/ycsb 25M rodam limpos no mesmo
box/binário — mas **não rodam o seed deps** (`need_seed` só para
apply_batch/mvcc_latest/scan/lock_prewrite): o seed são ~200M ops CF
(8/registro: lock, default×100B, write, lock-del × 2 rounds) ≈ 7–9 GiB
de SST.

`:p261d` re-roda com stderr capturado + sampler RSS 2s + smaps + dois
discriminadores: `ROCKS_PARITY_SETTLE=0` (seed+bench sem compact-settle)
e `PEDRA_SST_PAYLOAD_BUDGET=32MiB` (A/B do pool de payloads), mais
`deps_mvcc_latest` (mesmo seed, bench de ponto) condicionado.

Estruturas auditadas e **descartadas** como vazamento (todas limitadas):
payload pool 256 MiB (compat abre bounded), block cache 8192 entradas ×
`BLOCK_TARGET` 4 KiB ≈ 32 MiB, memtable auto-flush 4 MiB/CF,
`retired_pending` cap 16 MiB, `WARM_FLOOR_BYTES` 3 GiB é conselho de
page cache (DONTNEED), não alocação. Precedente no fonte: comentário em
`reads_served` — "keeping one BTree per L0 alive as a read cache during
bulk load OOMed a 4 GiB host at 25M entries" — e `can_admit`:
"hydrate leaves payloads empty (100M OOM otherwise)". OOM em escala é
modo de falha conhecido do engine; o 0,70× do cartaz veio da matriz
modelada (warm-cap), nunca de uma rodada física 4 GiB.

## P1 — o corte (a definir pelo DIAG)

Se (a): aquecimento/promoção de janela por família de prefixo no `count_named`.
Se (b): fusão do setup do iterador / caminho KeyOnly enxuto. Se o OOM
precede o bench em 100M (provável), **P1 vira memória primeiro**: caber
deps_scan 100M em 4 GiB é pré-requisito do pay físico da célula.

## P2 — fora

Readahead >1 bloco (inerte por construção aqui), cache de contagem maior
(já simétrico com Rocks).
