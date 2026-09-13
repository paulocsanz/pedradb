# P2.5 — Cobertura dos eixos sem número (banda mc9–49, célula 1 GiB, delete-heavy, p99/p999)

**Data:** 2026-09-13 (sweep 07:01–10:26 local) | **Caixa:** Darwin local
(**DIAG — nunca cartaz**) | **Binário:** árvore viva head `0ba886fe`
(build 07:01; **não inclui** `0eb0f25e` — probes de sub-fase do scan não
afetam comportamento, e os ratios acima não dependem deles; a emissão
ssu/smg vem de re-run com binário rebuilt no finding P2.6), braço rocks
sempre `--features real` em target próprio (`$SCR/p25-rocks-target`).

**Protocolo** (`$SCR/p25-sweep.sh`, quiet-gated load<4.0, 1 round por
célula — DIAG): (a) eixo mc 9/16/32/49 (`ROCKS_PARITY_CLIENTS=c`,
`ROCKS_YCSB_OPS=3000`, formas ycsb_a/f + deps_cache_overwrite/apply_batch/
raftlog, singles + `_mc{c}`); (b) célula **g1 = 1 GiB**
(`ROCKS_YCSB_RECORDS=10000000` ≈ 1 GiB working set, `OPS=5000`, suítes
ycsb+deps completas) — **primeira célula de escala medida contra rocks**
(no harness compat-vs-rocks; as células 4 GiB de P2.1 são harness
separado Pedra-only); (c) delete-heavy via `linkbench_mix`
(`ROCKS_PARITY_ONLY=linkbench_mix`, suíte myrocks = **peer host-default
sync=true ⇒ DIAG-only, nunca claim**). Peer das células (a)/(b) =
RocksDB default `sync=false` (JSON `sync:false`); coluna Pedra = async
same-class. p50/p95/p99/p999/max capturados em todas as formas
(`b7aaa0f0`).

## Tabela completa (ratio = qps Pedra / qps rocks; ≥1,0 = Pedra na frente)

### (a) Banda mc9–49 (fronteira que era gap mc8→mc50)

| forma | mc9 | mc16 | mc32 | mc49 |
|---|---|---|---|---|
| ycsb_a_mc | 0,329 | 0,290 | 0,303 | **4,029** (rocks p999 58,6ms) |
| ycsb_f_mc | 0,359 | 0,301 | 0,465 | 0,889 |
| deps_cache_overwrite_mc | 0,440 | 0,318 | 0,597 | **1,153** |
| deps_apply_batch_mc | 0,389 | 0,446 | 0,435 | 0,316 |
| deps_raftlog_mc | 0,381 | 0,328 | 0,312 | 0,300 |
| ycsb_a (single) | 0,022 | 0,499 | 0,152 | 0,557 |
| ycsb_f (single) | 0,139 | 0,561 | 0,580 | 0,639 |
| deps_apply_batch (single) | **1,431** | **1,605** | **1,467** | **1,349** |
| deps_raftlog (single) | **1,609** | **2,425** | **1,712** | **1,250** |
| deps_cache_overwrite (single) | 0,429 | 0,462 | 0,453 | 0,524 |

Singles ycsb_a/f são os ruídos conhecidos do DIAG Darwin (p99 pedra
0,007–0,146ms variando com o round); o sinal novo é a banda mc: **0,29–0,60
em TODAS as formas mc9–mc32**, subindo para ~1,0–4,0 em mc49 quando o
rocks começa a pagar cauda (ycsb_a_mc49 rocks p99 4,4ms / p999 58,6ms).

### (b) Célula g1 — 1 GiB, 10M records (DIAG Darwin)

| forma | ratio | pedra p50 | rocks p50 | pedra p99 | rocks p99 |
|---|---|---|---|---|---|
| ycsb_e (scan) | **0,001** | 8,773ms | 0,0125ms | 10,510ms | 0,021ms |
| deps_cache_overwrite | **0,037** | 5,9µs | 2,4µs | 9µs | 4µs |
| deps_scan | **0,045** | 0,202ms | 0,0124ms | 0,822ms | 0,019ms |
| deps_raftlog | 0,250 | 4,6µs | 6,0µs | 8µs | 10µs |
| deps_mvcc_latest | 0,281 | 21,3µs | 17,0µs | 0,284ms | 0,028ms |
| ycsb_a | 0,656 | 5,1µs | 3,2µs | 11µs | 8µs |
| ycsb_f | **1,041** | 6,6µs | 6,1µs | 13µs | 11µs |
| ycsb_c | **1,729** | 2,3µs | 4,2µs | 7µs | 9µs |
| ycsb_c_unif | **2,161** | 2,1µs | 4,3µs | 4µs | 10µs |
| ycsb_c_big | **2,292** | 1,1µs | 3,2µs | 5µs | 7µs |
| ycsb_d | **1,661** | 2,3µs | 4,3µs | 9µs | 9µs |
| ycsb_b_unif | **1,730** | 2,3µs | 4,3µs | 8µs | 9µs |
| ycsb_b | **8,784** | 2,3µs | 4,8µs | 10µs | 22µs (p999 3,89ms) |
| deps_apply_batch | **1,468** | 64,5µs | 129,6µs | 105µs | 206µs |
| deps_lock_prewrite | **1,567** | 31,5µs | 81,8µs | 44µs | 122µs |

### (c) Delete-heavy (linkbench_mix; **peer sync=true ⇒ DIAG-only**)

| forma | ratio | pedra p50 | rocks p50 | pedra p99 |
|---|---|---|---|---|
| linkbench_mix | 0,009 | 6,0µs | 2,0µs | 4,934ms |
| myrocks_write_tx | 0,024 | 4,030ms | 66,7µs | 17,389ms |
| myrocks_point_select | 0,901 | — | — | — |
| myrocks_read_only | **4,946** | — | — | — |

## Achados (cada célula nova <1,0 vira fatia datada — regra de extensão)

1. **Scan colapsa com a escala** (novo dono nº 1 do board de leitura):
   `ycsb_e` **7,976–10,046 WIN no gate Linux p201o @1024 records** →
   **0,001 @10M** (p50 8,77ms vs 0,0125ms — ~700×; p999 55ms). E
   `deps_scan` **1,023–1,419 DIAG @1024** (P2.3, pós-point_ord_btree) →
   **0,045 @10M** (16× no p50). Leitura de range sobre working set grande
   é a regressão dependente de escala mais violenta já medida no projeto.
   Small-scale parity não extrapola. → **fatia P2.6** (decomposição em
   10M com os probes `scan_sst_setup_ns`/`scan_merge_ns` + dono nomeado;
   ataque candidado = single-pass min-head stepped cursors, citado no
   P2.3, mais skip por bloco no k-way).
2. **Write-path degrada com a escala**: `deps_cache_overwrite` já era
   buraco aberto no gate Linux (p201o 0,512–0,647, dono **hat**
   pwrite-por-op ticket 0193) e despenca para **0,037 @10M** (14× pior);
   `deps_raftlog` Linux WIN 1,577–1,640 @1024 → **0,250 @10M**;
   `deps_mvcc_latest` Linux WIN 3,999–4,281 @1024 → **0,281 @10M**
   (pedra p99 284µs vs rocks 28µs — cauda, não p50);
   `ycsb_a` 0,605–0,905 Linux → 0,656 @10M. → **fatia P2.7** (meter de
   atribuição `write_phase_stats`/PHASE em RECORDS=10M — o dono @escala
   ainda não é o dono @1024; L0/SST count, flush, memtable BTree de 10M
   chaves são os suspeitos **hat**).
3. **Banda mc9–49 inteira <1,0 no DIAG** (0,29–0,60 em mc9–mc32;
   mc49 encosta: cache_overwrite_mc49 1,153, ycsb_f_mc49 0,889,
   ycsb_a_mc49 4,03 só porque o rocks paga p999 58ms). Consistente com a
   fronteira já adjudicada: a coluna async do Darwin não tem barreira
   in-flight para amortizar (P0.3b) e o Linux decide em P0.4/e4b. →
   **fatia P2.8** (re-adjudicação: dono = mesmo mecanismo da fronteira;
   veredito Linux = e4b, sem mecanismo novo a implementar).
4. **Delete-heavy (sync-peer DIAG)**: linkbench_mix 0,009 — p50 do mix
   é 6µs (não é o dono), o p99 4,934ms/p999 8,043ms é o commit
   write-side Darwin (`fcntl F_FULLFSYNC` 4,18ms, 80× estrutural,
   já decomposto no P1.4; não existe no Linux). `myrocks_write_tx`
   p50 4,03ms = 1 F_FULLFSYNC por commit idem. Alimenta P1.3/P1.4
   (cartaz = e4b); **nunca claim** (peer sync).
5. **Leitura paga em escala (DIAG)**: ycsb_b 8,784 (rocks p999 3,89ms —
   cache miss em 1 GiB), c/c_unif/c_big 1,7–2,3, d 1,661, b_unif 1,730,
   apply_batch 1,468, lock_prewrite 1,567, f 1,041. Point-gets com
   working set 1 GiB são o território mais confortável do Pedra —
   confirma o padrão kvrocks_get 4,7× do gate.

## Fatias abertas por este finding (regra de extensão do RFC-0217)

- **P2.6 scan-at-scale** (dono nº 1 novo): decompor ycsb_e/deps_scan
  @10M (probes já no código), nomear dono, atacar; alvo cartaz Linux
  1 GiB ≥1,0.
- **P2.7 write-at-scale**: atribuição de fase @10M (PHASE na célula g1)
  para raftlog/mvcc_latest/cache_overwrite/ycsb_a; dono nomeado → ataque.
- **P2.8 banda mc9–49**: veredito Linux = e4b (P0.4); DIAG fechado como
  fronteira contínua mc2→mc49 (nenhum ponto vira de perda para win
  antes de o grupo amortizar no Linux).

## Artefatos

JSONs e logs completos em `$SCR/p25-sweep/{mc{9,16,32,49},g1,lb}-{compat,rocks}/`
(recomputados neste README pelo driver python do scratch; tabela acima é
saída literal do recompute contra os JSONs).
