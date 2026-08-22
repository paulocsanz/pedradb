# 2026-08-22 — deps_raftlog pubfix: publish-phase invalidation era o gargalo nº1

**VERDICT: NON-OFFICIAL** — caixa suja o dia inteiro (load 6–12, 5ª/6ª
baterias sujas consecutivas; ver `../rfc0029-prefetch-n/`-era e rearm6).
Tudo aqui é A/B mesmo-binary-dir alternado (mesma caixa, mesmo ruído) ou
probe de fase; números absolutos valem como direção, não como oficial.

Alvo: `deps_raftlog` — **pior shape do catálogo**: **0,72×** limpo
(rearm6 r3: compat 93.426 vs rocks 129.246; `../2026-08-22-p23-rearm5/`
mostrou 0,85–1,03 sujo). Piso RFC-0041 P1.3 = ≥2×.

## Cadeia de diagnóstico (o que foi descartado antes de achar)

1. **Syscall/WAL-buffer — MORTO.** `ASYNC_WAL_BUFFER` 64 KiB → 1 MiB
   (throwaway): batch-16 flat (4,41→4,72 µs), batch-160 **pior**
   (31,98→47,79 µs — frame de 1 MiB cache-cold). O custo do WAL é
   encode CPU + tráfego de memória, não `write(2)`. O 64 KiB é paridade
   com `writable_file_max_buffer_size` do RocksDB e fica como está.
2. **Pisos micro** (`#[ignore]` landed junto com este finding):
   WAL `encode_write_op_batches` + buffer = **1,305 µs/batch**
   (0,0816/op); memtable `insert_many` = **3,148 µs/batch**
   (0,1967/op). Soma ≈ 4,5 µs vs probe total ~11 µs/batch → **~6,5 µs
   por batch fora dos pisos**.
3. **Curva de batch**: 1 op = 0,73 µs/op total; 16-op ~0,6 µs/op — o
   shape não é vítima de batching pequeno.
4. **Perfil de fase** (`raftlog_phase_probe`, `PEDRA_WRITE_PHASE_STATS=1`,
   load 12): a fase **publish** custava **3,17 µs/batch** — 28% do total,
   quase todo na invalidação pós-commit:
   - `CountCache::record_dirty`: **2 allocs `Box::from` + 2 inserts de
     hash por chave escrita, a cada publish, mesmo com o mapa vazio**
     (write-only não tem entry de count — o trabalho era 100% perdido);
   - `point_cache.invalidate` por chave: 1 lock de mutex por chave.
5. **Fold do park**: `fold_parked_once_off_lock` clona 2 MemTables
   profundos por fold e satura um núcleo ao lado do writer em escala
   ≥1M-batch (7,3k amostras em `clone_subtree`). **Não é o custo na
   perna oficial** (A/B NOFLUSH a 300k idêntico) — fica como candidato
   de escala longa, não de perna n=2000.

## Fix (landed `8e3a460` como F204)

- `CountCache`: watermark `skipped_below` — `record_dirty` com mapa
  vazio só avança o watermark (0 alloc); `get` rejeita entry com
  `seq < skipped_below`. Também **fecha a corrida F204**: leitor
  lock-free que insere entry stale após o skip não valida mais
  (regression test `count_cache_skip_while_empty_retires_racy_insert`).
- `AnswerCache::invalidate_many(&[Bytes])`: 1 lock, N removes —
  substitui o loop por-chave em `invalidate_read_answers` (db.rs).

## Evidência

### Probe de fase (load 12)

| fase | antes | depois |
|---|---|---|
| publish | 3,17 µs/batch | **0,15 µs/batch (−95%)** |
| total | 11,1 µs/batch | 9,3–10,0 µs/batch |

### A/B end-to-end — oficial-shape, 15 rounds alternados (n=2000, async, caixa suja)

- old = `b21379f` (worktree), new = working tree com o fix.
- `deps_raftlog`: **old med 47.647 / new med 59.329 = +24,5%**
  (JSONs em `ab/raftlog-{old,new}-{1..15}.json`).
- Extrapolação contra o limpo (rearm6 r3 93.426 × 1,245 ≈ **116k** vs
  rocks 129.246 ≈ **0,90×**) — direção, não oficial.
- `ycsb_a` dedicado 15 rounds: **+3,1%** (3.232.323 → 3.331.712) —
  o 0,82 do guard de 3 rounds era ruído da caixa suja.

### Guard de suítes (3 rounds, medianas, caixa suja)

apply 1,03 · mvcc 0,99 · b 0,96 · c 1,00 · d 1,02 · e 1,02 · f 1,01.
Bandeiras do guard de 3 rounds (scan 0,91, prewrite 0,96, a 0,82): `a`
foi refutado como regressão pelo dedicado 15 rounds acima; scan/prewrite
não tocam o caminho do fix (invalidação é write-side) e estão na banda
de ruído da caixa — sem dedicação extra nesta direção.

### Testes

`pedradb-core` release suite: **391 passou, 0 falhou**; probes/micros
landed `#[ignore]` (reprodutíveis).

## Onde continua o arco (0,90× → ≥2×)

Probe total 9,3–10 µs vs pisos ~4,5 µs: ainda ~2× de headroom.

1. **Fold deep-clone** (`fold_parked_once_off_lock`): Arc/take em vez
   de `(*a).clone()` — domina em escala ≥1M e em multi-cliente.
2. **Memtable `tail_idx`**: 0,197 µs/op de piso (BTreeMap ordenado,
   exigido por range-scan) — candidato a estrutura mais barata para
   key sem sobreposição.
3. **Submit-path**: overhead entre `Engine::batch` e
   `commit_async_ops` ainda não decomposto.

## Reproduzir

```sh
cargo test -p rocksdb-compat --release raftlog_phase_probe -- --ignored --nocapture
WAL_MICRO_OPS=16 WAL_MICRO_N=200000 cargo test -p pedradb-core --release wal_encode_raftlog_micro -- --ignored --nocapture
MEM_MICRO_OPS=16 MEM_MICRO_N=200000 cargo test -p pedradb-core --release mem_insert_raftlog_micro -- --ignored --nocapture
# A/B: ver formato em ab/summary.txt (ROCKS_PARITY_ONLY=deps_raftlog ROCKS_YCSB_OPS=2000 PEDRA_PARITY_ASYNC=1)
```

Baseline limpo para comparar: `../2026-08-22-p23-rearm5/` rounds
(0,85–1,03 sujo) e rearm6 r3 (0,72× limpo).
