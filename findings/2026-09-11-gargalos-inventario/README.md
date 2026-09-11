# Inventário de gargalos — toda célula <1× não paga (2026-09-11)

**Data:** 2026-09-11 | **Fontes:** `findings/2026-09-11-p201r2-mc4/`
(3 rounds quiet, imagem p201r2 digest `sha256:0ec55b38…`),
`findings/2026-09-11-p201o-sweep/` (20 formas, 3 rounds quiet + A/B p201q
digest `sha256:18babf02…`), tail do log `caixote logs linux-gate-p149b`
(recuperação `ycsb_a_mc4` r3). **Protocolo comum:** mesmo boot, coluna
same-class (`PEDRA_PARITY_ASYNC=1`), peer RocksDB default
`ROCKS_PARITY_SYNC=0` do mesmo round, min-of-3 = cartaz; Darwin = DIAG.
**Regra de leitura:** nenhuma linha single-client write é win; nenhum
ratio sync-peer é win; previsões rotuladas **hat**.

## A. Buracos medidos Linux — dono RFC-0209 (ataque async 1-op)

Mecanismo comum **verificado in-tree 2026-09-11** (não é hat): o caminho
async 1-op (`commit_async_one` db.rs L9358 → `write_pending_frame`
wal/mod.rs L210 → `write_frame` wal/writer.rs L143: `out.write_all(buf)`)
faz **uma syscall `write()` por operação**; o Rocks `sync=false` só faz
memcpy no buffer user-space do WritableFile e escreve quando o buffer
enche. 4 writers == ncpu ⇒ política 0201 despacha tudo por
bypass/`commit_async_one`. Ataque = buffer WAL user-space (staging no
`WalWriter`, flush por tamanho 64 KiB / antes-de-sync / no close;
**sem wait-to-grow** — 0180/0190 vetados; opt-in por env até meter).

| # | célula | min | mediana | rounds | observação |
|---|---|---:|---:|---|---|
| 1 | ycsb_f_mc4 | **0,2947** | 0,3195 | 0,2947/0,6338/0,3195 | rmw = get + write 1-op; get soma em cima do buraco de write |
| 2 | deps_cache_overwrite_mc4 | **0,3698** | 0,4611 | 0,3698/0,4611/0,4722 | 4 writers 1-op; rocks 173k–349k qps (bufferizado) vs pedra 82k–129k |
| 3 | ycsb_a single | **0,605** | 0,905 | sweep p201o | 50% write 1-op no mix; lado leitura PAGO (ycsb_c 2,729) ⇒ buraco é write-side (**hat** de atribuição; medidor 0209 confirma) |
| 4 | deps_cache_overwrite single | **0,512** | 0,647 | sweep p201o | 1-op single-client, mesma classe |
| 5 | kvrocks_set single | **0,748** | 0,757 | sweep p201o | 1-op single-client (a variante mc50 está PAGA 1,678× — grupo commit fecha; o single não) |
| 6 | ycsb_f single | **0,804** | 0,956 | sweep p201o | rmw single-client |
| 7 | ycsb_a_mc4 | **0,336** (1 round) | — | r3=0,3362 (pedra 156 121 vs rocks 464 445 qps) | recuperado do TAIL do log do caixote (janela 500 entradas; r1/r2 rolaram fora) — **rotulado: round único, DIAG-grade**; a onda de meter do 0209 mede 3/3 |

**Dono de A: RFC-0209 P0** (buffer WAL user-space). Células-guardião do
meter: `deps_cache_overwrite_mc4` + `ycsb_f_mc4` (+ single-client
`kvrocks_set`, `deps_cache_overwrite`) e a célula 10k pendente (ver C).

## B. Buraco medido Linux — dono leitura (GET-side)

| # | célula | min | mediana | dono/ataque |
|---|---|---:|---:|---|
| 8 | deps_scan single | **0,831** | 0,922 | scan bounded-cache: 0195 P0.1–P0.3 (WILLNEED janela) aterrizado sem meter 100M; nesta escala (1024 records) o custo por bloco do scan segue > iterator Rocks. Ataque: decompor o ciclo do cursor (decode por bloco vs iterator) — **hat**, precisa perna de telemetria scan-side; deferral-com-número até lá. Não é dono do 0209 |

## C. Células pesadas / sem número — deferral com custo nomeado

| # | célula | número | estado |
|---|---|---|---|
| 9 | prefix 100M @4 GiB | **0,70×** (cartaz 09-10) | corte 0195 aterrizado; meter com/sem (0195 P0.4) **deferred** — custo: dataset 4 GiB + build/bench por braço ≈ horas de gate; reabre quando uma onda de gate tiver orçamento (0196 P1.1) |
| 10 | overwrite_mc4 25M @4 GiB | **0,557×** 3/3 (quieto 09-09) | mecanismos write (0193) + leftover (0194) aterrizados sem meter; meter 15M+25M **deferred** com o mesmo custo (0196 P1.2) |
| 11 | overwrite_mc4 10k | min 0,883 (trim7+8, **pre-0193**) | gate 0185 P0.3 (3/3 ≥ 1,0) segue SEM número Linux pós-0193 — **dono: onda de meter do 0209** (`ROCKS_YCSB_RECORDS=10000`) |
| 12 | Grid B (10–100×, compaction on) | — | deferred; atrás de um corte vencedor vivo (nenhum corte novo aterrizou desde a escrita; 0196 P2.2) |

## D. U-cells DIAG-only (sem Linux 3-run — regra anti-overfit)

qs_neg 0,808; point_select 0,430; wbwi 0,410; flink 0,522; venice 0,735;
arango 0,003; pipelined 0,807 (todos Darwin DIAG). **Nenhuma recebe
mecanismo sem Linux 3-run** (0196 P2.1 parcial: a família ycsb+unif FOI
medida no sweep p201o; estas não). Dono: lote futuro de medição; nenhuma
exceção aberta.

## E. Contexto — pagas neste ciclo (não são buracos; não re-medir)

apply_mc4 **1,0859×** (p201r2); kvrocks_set_mc50 **1,678×** (p201q A/B);
ycsb_b 1,088 / b_unif 1,394 / c 2,729 / c_unif 3,238 / d 1,122 / e 7,976;
kvrocks_blob_set 2,351 / kvrocks_get 4,738 / kvrocks_pipelined_set 1,203 /
kvrocks_scan 26,403; deps_apply_batch 1,302 / deps_lock_prewrite 1,659 /
deps_raftlog 1,577 / deps_mvcc_latest 3,999 (sweep p201o).

## Conclusão

O maior buraco vivo medido é **uma única classe de mecanismo** — syscall
`write()` por operação no caminho async 1-op — aparecendo em 7 células
(A1–A7), com âncoras min-of-3 em mc4 (0,295/0,370) e single (0,51–0,80).
O RFC-0209 ataca essa classe no tamanho dela: staging no `WalWriter`
(flush por tamanho, antes de sync, no close; ordem global preservada por
flush-antes-de-escrita-direta; sem espera por writers). O resto do board
está pago, é read-side (B), pesado-deferred (C) ou DIAG-only (D).
