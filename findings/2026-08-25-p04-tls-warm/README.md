# RFC-0062 P0.4 — write-through TLS no `get idx-1` (Darwin proxy)

**Corte:** depois de `write_cf_owned` / `put` / `put_cf`, as chaves vão para
`LAST_CF` / `LAST_GET` na epoch publicada. O `get_named` do `idx-1` (a
cada 8 ops no `deps_raftlog`) não faz encode+lock+walk — diag-6e: esse
get sujava o I-cache do `batch()` seguinte (NOGET 1.18×, shape oficial
0.96×).

Também: `IoUringFile::write` no `main` passa a `pwrite(2)` (já estava no
worktree 17d10c4; P0.3 Linux mediu isso).

**Não é Linux.** 3 rounds neste Mac, load 8–10, `ROCKS_PARITY_ONLY=deps_raftlog`
ainda correu YCSB no mesmo processo (mem_entries=3484 no enter). Proxy
de direção, não o gate P0.3.

| r | ratio | p50 µs P/R | p99 µs P/R | qps P/R |
|---|---:|---|---|---|
| 1 | **1.975** | 5.2 / 8.8 | 33.6 / 33.2 | 154k / 78k |
| 2 | **1.704** | 5.2 / 8.7 | 30.9 / 20.5 | 151k / 89k |
| 3 | **1.675** | 5.7 / 9.0 | 31.8 / 20.1 | 145k / 87k |

**min 1.675.** p50 nosso ~5.2 vs Rocks ~8.8. p99 Linux (P0.3 isolado 32 vs
18) **não** foi re-medido.

Teste: `write_cf_owned_warms_named_get_tls`.

Próximo: mesma bateria P0.3 no caixote 4 vCPU com este binário.
