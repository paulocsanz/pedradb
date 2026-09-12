# Inventário de gargalos — toda célula <1× não paga (2026-09-11, rev. 2 pós-p209b)

**Data:** 2026-09-11 (rev. 2: mesma noite do meter p209b) | **Fontes:**
`findings/2026-09-11-p201r2-mc4/` (3 rounds quiet, imagem p201r2 digest
`sha256:0ec55b38…`), `findings/2026-09-11-p201o-sweep/` (20 formas, 3 rounds
quiet + A/B p201q digest `sha256:18babf02…`),
`findings/2026-09-11-p209-wal-buffer-meter/` (p209a/p209b, same-boot,
braços buf/nobuf, imagem p209b digest `sha256:449b4b68…`, src = `c064d621`),
tail do log `caixote logs linux-gate-p149b`. **Protocolo comum:** mesmo boot
para toda comparação, coluna same-class (`PEDRA_PARITY_ASYNC=1`), peer
RocksDB default `ROCKS_PARITY_SYNC=0` do MESMO round, min-of-3 = cartaz;
Darwin = DIAG. **Regra de leitura:** nenhuma linha single-client write é
win; nenhum ratio sync-peer é win; previsões rotuladas **hat**; cross-boot
absoluto NÃO é comparável (nota de re-ancoragem do p209b).

## A. Buraco estrutural nº 1 — escalonamento rmw mc4 (`ycsb_f_mc4`)

Mecanismo (código + dois meters independentes, não hat): `ycsb_f_mc4` =
4 clientes × ops {50% `get_probe` + 50% rmw (`get` + `put` 1-op)}. O `put`
1-op async no regime writers == ncpu vai pelo **bypass** 0201
(`client_axis_kernel::async_merge_policy`: merge só quando writers >
ncpu): cada writer toma a write-lock do Db e corre o commit INTEIRO
serializado — `encode_async_one` + `write_pending_frame` (syscall) +
`apply_async_one` (mem+publish+flush-check) — enquanto o Rocks
`sync=false` só paga memcpy num buffer user-space por writer.

Dois degraus medidos same-boot (p209b, 3 rounds quiet):

| braço | ycsb_f_mc4 min | leitura |
|---|---:|---|
| nobuf (default) | **0,532** | syscall `write()` por op DENTRO da write-lock do Db |
| buf (`PEDRA_WAL_BUFFER=1`) | **0,780** | syscall sai do caminho; resta a serialização encode+apply+publish sob a lock |

O buf **aperta a dispersão** (nobuf 1,882/0,791/0,532; buf 0,963/1,055/0,780)
e levanta o piso +47% sem fechá-lo ⇒ o dono do resto é o **escalonamento**
(a lock e a série serializada), não a syscall isolada. Células-irmãs no
mesmo boot: ycsb_a_mc4 nobuf 1,537 (o mix 50/50 sem dependência get→put
não afunda), deps_cache_overwrite_mc4 nobuf 1,035 (write puro 1-op
recupera ≥1 no mesmo boot). **A forma que afunda é rmw**: o `get` do rmw
toma a read-lock entre duas write-locks do mesmo thread + o `put` paga a
série serial completa.

| # | célula | número (rótulo) | dono |
|---|---|---|---|
| 1 | ycsb_f_mc4 | **0,532** min same-boot p209b nobuf / **0,780** buf / 0,2947 cross-boot p201r2 | **RFC-0211** (escalonamento rmw mc4) |
| 2 | ycsb_f_mc4 buf-arm residual | 0,780 min (p209b) | RFC-0211 (mesma fatia: serialização pós-syscall) |
| 3 | ycsb_a_mc4 buf-arm | 1,115 min (p209b; nobuf 1,537) | RFC-0211 guarda — regressão do buf a não propagar |

Âncoras do RFC-0211 P0: piso **0,532** (nobuf) / braço buf **0,780** /
alvo **≥1,0** same-class async same-boot. Mecanismos candidatos a
adjudicar por meter (todos JÁ existem como env no caminho real, zero
código para medir): `PEDRA_ASYNC_GROUP=1` (merge do grupo no regime
writers==ncpu: líder drena a geração — 1 frame/1 write off-lock, pago no
mc50 1,678×), `PEDRA_WRITE_FAIR=1` (handoff dirigido no unlock do
bypass), `PEDRA_WRITE_SPIN=N` (spin-then-park), e a composição com
`PEDRA_WAL_BUFFER=1`.

## B. Buracos medidos — geração de âncoras cross-boot (p201r2/p201o)

Estas âncoras são o cartaz histórico (boot das ondas p201*); o p209b
re-ancorou same-boot o subconjunto dele (nota de re-ancoragem: overwrite
single 2,78 vs âncora 0,512; ycsb_a single 2,60 vs 0,605; overwrite_mc4
1,035 vs 0,370 — cross-boot absoluto não é comparável; a hierarquia
same-boot viva está na seção A/C):

| # | célula | min | mediana | rounds | estado 2026-09-11 |
|---|---|---:|---:|---|---|
| 4 | deps_cache_overwrite_mc4 | 0,3698 (p201r2) | 0,4611 | 0,3698/0,4611/0,4722 | same-boot p209b: 1,035/1,026 — re-ancorado ≥1; vira guarda do 0211 |
| 5 | ycsb_a single | 0,605 (p201o) | 0,905 | sweep | **hat A3 FECHADO**: 2,602 min neste boot (p209b) — perda era boot-specific |
| 6 | deps_cache_overwrite single | 0,512 (p201o) | 0,647 | sweep | same-boot 2,783/2,860 — re-ancorado ≥1 |
| 7 | kvrocks_set single | 0,748 (p201o) | 0,757 | sweep | same-boot 2,158/2,448 — re-ancorado ≥1 (buf +15% med) |
| 8 | ycsb_f single | 0,804 (p201o) | 0,956 | sweep | same-boot 1,881/1,094 — buf REGREDIU o min (1,881→1,094): razão do P1.1 não-flip |
| 9 | ycsb_a_mc4 | 0,336 (1 round, DIAG-grade, tail do log) | — | r3 | same-boot 1,537/1,115 — re-ancorado ≥1 |

## C. Células pesadas / sem número Linux — deferral ou gate com nome

| # | célula | número | estado (datado) |
|---|---|---|---|
| 10 | prefix 100M @4 GiB | **0,70×** (cartaz 09-10) | corte 0195 aterrizado sem meter; **blocked re-adjudicado 2026-09-11**: gate 09-10 reaberto 01:30, bloqueio = orçamento de onda + custo (dataset 4 GiB + ≈horas/braço; host-check 09-11: Darwin dados 94%/61 GiB livres — ENOSPC 09-06/09-10 não persiste; load1 10,78); onda do ciclo = P0 do 0211; dono 0196 P1.1 |
| 11 | overwrite_mc4 25M @4 GiB | **0,557×** 3/3 (quieto 09-09) | mecanismos 0193/0194 aterrizados sem meter; mesmo blocked re-adjudicado (dono 0196 P1.2) |
| 12 | overwrite_mc4 15M @4 GiB | — | idem (mesma onda do 25M) |
| 13 | overwrite_mc4 10k | min 0,883 (trim7+8, pre-0193) | **PAGA 2026-09-11 (p209b)**: gate 0185 P0.3 3/3 ≥1,0 nos dois braços (nobuf 1,078 / buf 1,268 min; peer 200k–307k saudável) |

## D. U-cells DIAG-only — lote Linux 3-run MEDIDO 2026-09-11/12 (p211u3, atualização pós-gate)

Predecessores Darwin DIAG: qs_neg 0,808; point_select 0,430; wbwi 0,410;
flink 0,522; venice 0,735; arango 0,003; pipelined 0,807. **Lote Linux
3-run quiet min-of-3 aterrissou** (`findings/2026-09-11-p209-ucells-gate/`,
suítes opt-in do harness, 21/21 células): **15 ≥1,0** (qs 1,345–1,763;
myrocks_point_select 1,162; read_only 4,123; flink 1,544; arango
1,237/3,346; venice 3,431; mixgraph 1,394; kvrocks 2,26–29,1) — o DIAG
Darwin subestimava todas as que viraram win; **6 perdas honestas nomeadas**:
kafka_changelog_flush 0,036 (flush-por-op: pipeline completo por flush —
parente medido do observável de contenção `compact_gate` do diagnóstico
p211u2), ingest_sst 0,069 e compaction_filter_drop 0,081 (rocksapi: APIs
nativas Rocks emuladas no compat), linkbench_mix 0,236, wbwi 0,494,
myrocks_write_tx 0,751. Dono de fatia futura se atacadas; nenhuma exceção
aberta.

## E. Outras frentes nomeadas (nada ignorado)

- **apply serial / teto 1c** (0183): apply_mc4 PAGO (1,0859 p201r2;
  1,466/1,473 p209b guards); o teto 1c G1 é por construção (fd antes de
  Ok) e nunca é win — linha G1 não cotada.
- **scan_readahead** (0195): kernel+wiring aterrizados; célula 100M
  blocked (C10). `deps_scan` single 0,831 (p201o) — read-side, decompor
  cursor é fatia própria (hat), não dono do 0211.
- **keep_budget / leftover** (0194): aterrizado; célula 25M blocked (C11).
- **l0 stall** (0167): parks contados (`rfc0167_l0_stall_parks_until_
  worker_drains`); workspace-test ENOSPC do host (disco 97–100%) era o
  bloqueio histórico — hoje 94%/61 GiB, re-testável.
- **ENOSPC-gated**: nenhum meter pesado roda com o host em pressão de
  disco; re-check datado acima (09-11).

## F. Telemetria viva (o que a onda do 0211 captura por rodada)

- `write_phase_stats` (linha `WRITEPHASE`/snapshot 7 contadores:
  prepare/wal/mem/publish/flush_check/lock_wait por commit) — separa
  lock_wait de wal no braço buf vs nobuf.
- `write_group_stats` (grupos, membros, bytes) — confirma merge vs bypass
  por braço (`PEDRA_ASYNC_GROUP`).
- `read_probe` JSON (mem_hit/sst_fallback/l0_files…) — lado leitura do rmw.
- `PEDRA_IO_ADVISE_STATS` (`scan_readahead=`/`leftover_advise=`) — fora do
  caminho write, zero por default (0169).
- Cartazes pagos NÃO re-medidos: apply_mc4 1,0859; kvrocks_set_mc50 1,678
  (guardas nas ondas novas em nível ≥, sem contradição — ver p209b).

## G. Contexto — pagas neste ciclo (não são buracos; não re-medir)

apply_mc4 **1,0859×** (p201r2); kvrocks_set_mc50 **1,678×** (p201q);
ycsb_b 1,088 / b_unif 1,394 / c 2,729 / c_unif 3,238 / d 1,122 / e 7,976;
kvrocks_blob_set 2,351 / kvrocks_get 4,738 / kvrocks_pipelined_set 1,203 /
kvrocks_scan 26,403; deps_apply_batch 1,302 / deps_lock_prewrite 1,659 /
deps_raftlog 1,577 / deps_mvcc_latest 3,999 (sweep p201o); célula 10k do
gate 0185 P0.3 (C13, p209b).

## Conclusão (rev. 2)

O buraco estrutural vivo nº 1, medido same-boot em dois degraus
(0,532 sem syscall no caminho → 0,780 com), é o **escalonamento do grupo
rmw no regime writers == ncpu** (herança da fronteira 0201): bypass
serializando o commit inteiro sob a write-lock do Db. O pipeline
drenável (líder + geração em 1 frame, write off-lock, group apply) já
existe, está pago no regime oversubscribed (mc50 1,678×) e não alcança o
regime ==ncpu. O RFC-0211 dimensiona o P0 contra 0,532/0,780/≥1,0 com
meter 3-run quiet min-of-3 e veredito datado (perda honesta vira fatia).
O resto do board: pago (G), read-side (E), pesado-blocked-com-gate-datado
(C) ou DIAG agendado para Linux 3-run (D).
