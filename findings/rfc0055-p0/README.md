# RFC-0055 P0 — pipeline_gap (NON-OFFICIAL)

Box not gated. Numbers decide whether to implement Rocks write-pipeline knobs.

```
cargo run --release -p rocksdb-parity-bench --example pipeline_gap
```

2026-08-23, this Mac, `PGAP_OPS=4000` `PGAP_APPLY=400`:

| writers | qps | note |
|---|---:|---|
| 1 | 18 420 | official ycsb/deps/kvrocks shape |
| 12 | 37 208 | 2× not 12× — lock/scheduling |
| 50 | 151 918 | still scaling; not mem-insert bound at 1c |

| apply flush | qps | sst | note |
|---|---:|---:|---|
| none | 11 049 | 0 | mem grows |
| **4 MiB** (product) | 5 863 | 3 | drain every 16 iters — **flush tax** |
| 64 MiB (Rocks default) | 20 553 | 0 | no flush in 400 iters |

## Veredito

- **Concurrent memtable / pipelined write:** as formas oficiais são 1 cliente.
  Não há segundo writer para paralelizar. Implementar skiplist não mexe no
  cartaz 13/18. mc50/mc4 é outra coluna (RFC-0045: hold 1.6 µs, wait 175 µs).
- **Multi-flush / 64 MiB default:** nesta janela 4 MiB *paga* flush e perde
  para 64/none. O bench oficial pina **256 MiB** (zero flush). O default 4 MiB
  é escolha de **produto** no apply longo com drain (histórico 2251 vs 1228
  qps em 2000 iters) — não vaza para o cartaz. Host que quer 64 chama
  `set_write_buffer_size`.

P1.1–P1.3 do RFC-0055 ficam `todo` até uma forma **oficial ou mc4/mc50**
mostrar o mem-apply como o gap. Hoje o número não obriga.
