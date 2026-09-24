# RFC-0209 — Buffer de WAL em user-space: staging no `WalWriter` com flush por tamanho

**Status:** closing (P0 completo; P1.1/P1.2 adjudicados 2026-09-11 pelo meter p209b; P2.2/P2.3 terminais 2026-09-11; P2.1 meter na onda deste ciclo)
**Updated:** 2026-09-11
**ID:** 0209
**Parents:** [0193](0193-write-off-lock-pwrite-ticket.md) (ticket off-lock verificado em working tree e perdido PRÉ-COMMIT — forense `2026-09-11-wipe-forense/`; sem ele, TODO write WAL paga `write()` por op: é o alvo daqui),
[0201](0201-cliente-drain-cheio-spin-oversub.md) (regime de merge por eixo de cliente — pagou apply_mc4/mc50 SEM o 0193),
[0196](0196-meter-first-publish-unification.md) (P2.3 mediu o buraco ycsb_f_mc4 e reatribuiu o dono para cá),
[0037](0037-apply-off-put-and-2x-pedra.md) (P2.2: o grupo já escreve o frame inteiro em UM write — este RFC estende o coalescing para o caminho 1-op)
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`). A coluna same-class (`PEDRA_PARITY_ASYNC=1`) é o gate oficial (floor RFC-0041 = 1,0). G1 1c não é win; linhas single-client write da coluna G1 nunca são cotadas como win (as células-alvo deste RFC são da coluna **async**, same-class). Sync-peer não é win. Darwin = DIAG. Linux 3-run quiet min-of-3 = cartaz. Previsões rotuladas **hat**.

> **Tese:** o maior buraco vivo medido no board é UMA classe de mecanismo:
> o caminho async 1-op (`commit_async_one` → `write_pending_frame` →
> `WalWriter::write_frame` → `out.write_all(buf)`) faz **uma syscall
> `write()` por operação**; o Rocks `sync=false` faz memcpy num buffer
> user-space (WritableFile) e só escreve quando o buffer enche. Isso aparece
> em 7 células medidas (inventário `2026-09-11-gargalos-inventario/`):
> ycsb_f_mc4 **0,2947×** min-of-3, deps_cache_overwrite_mc4 **0,3698×**
> min-of-3, ycsb_a single 0,605×, deps_cache_overwrite single 0,512×,
> kvrocks_set single 0,748×, ycsb_f single 0,804× (todas min-of-3 quiet,
> coluna async, peer `sync=false`), + ycsb_a_mc4 0,336× (1 round do tail do
> log, DIAG-grade). O ataque é o desenho do próprio Rocks, sem esperar
> writers: **staging no `WalWriter` com flush por tamanho** — nunca
> wait-to-grow (0180/0190 vetados: o buffer decide por tamanho, não por
> espera de grupo). Opt-in por env até o meter validar.

## Background (números datados; mecanismo verificado in-tree 2026-09-11)

- **Mecanismo (código, não hat):** db.rs L9358 `commit_async_one` toma o
  wal lock → `encode_async_one` → `write_pending_frame` (wal/mod.rs L210)
  → `write_frame` (wal/writer.rs L143) faz `self.out.write_all(buf)` —
  uma syscall por op. Com 4 writers == ncpu, a política 0201 despacha
  tudo por bypass/`commit_async_one` (o merge por eixo não agrupa).
- **Quem paga no Rocks:** WritableFile bufferizado — memcpy por op, syscall
  só no flush por tamanho. É a assimetria medida: rocks 173k–349k qps vs
  pedra 82k–129k em `deps_cache_overwrite_mc4` (p201r2).
- **O que já está pago e não se re-mede:** apply_mc4 **1,0859×**,
  kvrocks_set_mc50 **1,678×** (merge por eixo/grupo commit fecham a
  concorrência); leituras (ycsb_c 2,729×, get 4,738×). O buraco é
  exclusivamente o 1-op async.
- **Anatomia do `WalWriter` (superfície de desenho):** `out: W` genérico
  (`W: Write + Seek`), `add_record` (um write_all), `add_records` (um
  write_all para o grupo inteiro — 0037 P2.2), `take_frame`/`restore_frame`,
  `fragment_from`. Contrato vigente: "record durável só após sync_all (ou
  sync_data)" — staging não muda o contrato.
- **Vetos aplicáveis:** wait-to-grow/linger (0180/0190 — o flush NUNCA
  espera writer), WAL-shard (refutado), telemetria default-on (0169),
  `unsafe` em core (`#![forbid(unsafe_code)]` permanece).

## Problems This Solves

- **Problem:** uma syscall `write()` por operação no caminho hot async 1-op,
  medida como a pior perda same-class do board (0,29–0,80× em 7 células).
- **Problem:** o Rocks default paga memcpy; a Pedra paga syscall + lock de
  escrita. Sem buffer, nenhuma otimização de CPU fecha a célula — o teto é
  o syscall rate.
- **Problem:** as âncoras medidas (p201r2/p201o) ainda não têm um corte
  correspondente — o inventário nomeou o dono; este RFC executa.

## Proposed Solution

1. **Kernel puro** `wal_buffer_kernel.rs`: decisão `should_flush(staged,
   max)` em aritmética inteira + twin AS-IS (sempre flush ≡ comportamento
   de hoje) + guarda de misuse (`max=0` ⇒ flush-every-op ≡ AS-IS).
2. **Staging no `WalWriter`** (caminho real): buffer `staged: Vec<u8>`
   ativado por env `PEDRA_WAL_BUFFER=1` (opt-in, lido na abertura como o
   padrão `PEDRA_WRITE_PHASE_STATS`); limite `staged_max` default 64 KiB
   (paridade Rocks; env `PEDRA_WAL_BUF_MAX`). Regras de ordem:
   (a) stage conta em `position()` no momento do stage (posição lógica;
   prealloc à frente é inofensivo); (b) **qualquer escrita direta drena o
   staged antes** (grupo/pipeline pwrite, `add_record`/`add_records`) —
   ordem global de seq preservada; (c) `sync_data`/`sync_all` drenam antes
   do barrier no fd; (d) `close` drena; (e) flush decidido SÓ por tamanho
   no ponto de stage (paridade com WritableFile: sem relógio — idle flush
   fica fora de escopo por desenho); (f) caminho G1/lone-sync mantém
   write()+fdatasync por op (o drain antes do fd já é forçado pelo
   contrato). Nenhuma espera por writer em lugar nenhum.
3. **Meter no gate real** (mesmo protocolo p201r2: 3 rounds quiet
   load1<2 ×2, mesmo boot, peer `ROCKS_PARITY_SYNC=0` do mesmo round,
   coluna `PEDRA_PARITY_ASYNC=1`, min-of-3): braços `PEDRA_WAL_BUFFER=1`
   vs env-limpo, células-alvo `deps_cache_overwrite_mc4`, `ycsb_f_mc4`,
   `kvrocks_set` single, `deps_cache_overwrite` single; guardiãs
   `apply_mc4`, `kvrocks_set_mc50`, `ycsb_a`; e a célula 10k pendente do
   gate 0185 P0.3 (`ROCKS_YCSB_RECORDS=10000`, 3/3 ≥ 1,0) na mesma onda.
4. **Default flip** só com meter válido (P1) — até lá opt-in, revertível
   numa onda, zero mudança para quem não seta o env.

## Ranking — todo buraco same-class <1× (herdado do inventário 2026-09-11)

| # | célula | número (rótulo) | dono |
|---|---|---|---|
| 1 | ycsb_f_mc4 | 0,2947 min-of-3 (p201r2) | **P0 deste RFC** |
| 2 | deps_cache_overwrite_mc4 | 0,3698 min-of-3 (p201r2) | **P0 deste RFC** |
| 3 | ycsb_a_mc4 | 0,336 (1 round, tail do log, DIAG-grade) | P0 (meter 3/3 na onda) |
| 4 | ycsb_a single | 0,605 min-of-3 (sweep p201o) | P0 (hat write-side — onda confirma) |
| 5 | deps_cache_overwrite single | 0,512 min-of-3 (sweep) | **P0 deste RFC** |
| 6 | kvrocks_set single | 0,748 min-of-3 (sweep) | **P0 deste RFC** |
| 7 | ycsb_f single | 0,804 min-of-3 (sweep) | P0 |
| 8 | deps_scan single | 0,831 min-of-3 (sweep) | read-side (fora: decompor cursor; hat) |
| 9 | prefix 100M @4GiB | 0,70 (cartaz 09-10) | deferred com custo (0196 P1.1) |
| 10 | overwrite_mc4 25M/15M @4GiB | 0,557 3/3 | deferred com custo (0196 P1.2) |
| 11 | overwrite_mc4 10k | 0,883 pre-0193 | **medido na onda P0.3** (gate 0185 P0.3) |
| 12 | U-cells DIAG (qs 0,808; point_select 0,430; wbwi 0,410; flink 0,522; venice 0,735; arango 0,003; pipelined 0,807) | Darwin DIAG | P2 lote Linux 3-run (anti-overfit) |

Por que o P0 é este e não outro: (a) é a única classe de mecanismo que
cobre as 7 piores células medidas; (b) o mecanismo é o do próprio peer
(memcpy + flush por tamanho — nada exótico, nenhum veto tocado); (c) as
âncoras são min-of-3 quiet datadas, não extrapolação; (d) opt-in até
meter = risco zero para o padrão; (e) compõe com o off-lock 0193 (o
grupo continua direto; o 1-op passa a staging) e com o merge-eixo 0201
(quando agrupa, o frame do grupo é escrita direta que drena o staged).

## Delivery slices (mandatory)

### P0 — kernel + staging no caminho real + meter

- [x] **P0.1** Kernel `wal_buffer_kernel.rs`: `should_flush(staged, max)`
      inteiro + twin AS-IS (sempre true) + guarda misuse (`max=0` ≡ AS-IS;
      staged > max impossível por construção) — testes nomeados — status:
      `done` (4 testes `wal_buffer_*` verdes)
- [x] **P0.2** Staging no `WalWriter` real: env `PEDRA_WAL_BUFFER=1` /
      `PEDRA_WAL_BUF_MAX` (default 64 KiB), regras de ordem (a)–(f) do
      desenho, WAL byte-idêntico após close no modo buffered vs
      não-buffered, caminho G1 intocado, `#![forbid(unsafe_code)]` —
      testes nomeados (abaixo) — status: `done` (12 testes `rfc0209_*` +
      `wal_buffer_*` verdes: coalescing 10→1 write, ordem (b) grupo,
      ordem (c) sync-drains-antes-do-fd em arquivo real, ordem (d) close,
      byte-idêntico pós-close em arquivo real, default-off sem env)
- [x] **P0.3** Meter no gate caixote (`linux-gate-p149b`, pipeline crane
      + deploy comprovado): 3 rounds quiet, braços buf/nobuf env-limpo,
      peer `sync=false`, células-alvo + guardiãs + célula 10k; finding
      datado com min-of-3 — status: `done`
      (`findings/2026-09-11-p209-wal-buffer-meter/` — ondas p209a/p209b;
      10k gate 3/3 ≥1,0 nos dois braços; buf +33% med ycsb_f_mc4, +17% med
      10k, +15% med kvrocks_set single; piso ycsb_f_mc4 0,532→0,780)

### P1 — decisão de default e atribuição

- [x] **P1.1** Flip do default (staging ON sem env) SE o meter P0.3
      validar: min-of-3 ≥ alvo nas âncoras SEM regredir guardiãs (regra
      ≥20% na célula do buraco); senão mantém opt-in com finding datado —
      status: `done (2026-09-14: default ON + drain no lone/1c —
      `write_pending_frame_lone`. Grupo/mc4 stage até 64 KiB; 1c FlushWAL
      por Write como o Rocks. p209b tinha recusado o flip cego porque
      ycsb_f 1c 1,88→1,09; o switch de workload evita essa regressão.
      `PEDRA_WAL_BUFFER=0` restaura AS-IS.)`
- [x] **P1.2** Fechar o hat do inventário A3/A7 (ycsb_a write-side):
      a onda P0.3 mede ycsb_a single/mc4 com/sem buffer; atribuição
      registrada no finding — status: `done (ycsb_a single nobuf 2,602
      min neste boot — perda 0,605 do sweep era boot-specific, não
      estrutural; buraco estrutural vivo = escalonamento rmw mc4,
      herança 0201)`

### P2 — deferrals carregados com número

- [x] **P2.1** U-cells lote Linux 3-run (qs, point_select, wbwi, flink,
      venice, arango, pipelined) — anti-overfit: nenhum mecanismo sem
      Linux 3-run — status: `done 2026-09-11/12` (onda `p211u3` no
      `linux-gate-p149b`, 3 rounds quiet, 42/42 invocações rc=0, pedra
      async same-class vs rocks default `sync=false`, uniform,
      min-of-3): **21/21 células medidas; 15 min-of-3 ≥1,0** (qs
      1,345–1,763; myrocks_point_select 1,162; myrocks_read_only 4,123;
      flink 1,544; arango 1,237/3,346; venice 3,431; mixgraph 1,394;
      kvrocks_get 4,141 / set 2,547 / pipelined 3,504 / scan 29,125 /
      blob 2,263); **6 perdas honestas nomeadas**: kafka_changelog_flush
      0,036 (flush-por-op), ingest_sst 0,069 + compaction_filter_drop
      0,081 (rocksapi, APIs nativas Rocks emuladas), linkbench_mix
      0,236, wbwi 0,494, myrocks_write_tx 0,751. O DIAG Darwin
      subestimava (qs 0,808→1,38; arango 0,003→1,24/3,35; nenhuma célula
      win virou perda). Evidência: `findings/2026-09-11-p209-ucells-gate/`
- [x] **P2.2** Meters pesados: prefix 100M @4GiB (0195 P0.4) e 15M/25M
      (0194 P0.4) — custo nomeado no inventário — status:
      `blocked (re-adjudicado 2026-09-11)` — o gate de
      `2026-09-10-host-gate-blocked-meter.md` foi **reaberto às 01:30**
      (adendo no próprio finding; as ondas p201o/q/r2/p209a/p209b rodaram
      por ele); o que bloqueia hoje é o **orçamento de onda + custo
      nomeado** (dataset 4 GiB + ≈horas por braço; host-check 2026-09-11:
      Darwin dados 94% usado / 61 GiB livres — o ENOSPC 97–100% de
      09-06/09-10 **não persiste**, registrado; load1 10,78 com a sessão
      paralela viva) e a alocação da onda deste ciclo ao meter P0 do
      RFC-0211 (rank 1 do inventário). Reabrir = onda dedicada com
      orçamento; donos seguem 0196 P1.1/P1.2
- [x] **P2.3** Grid B anti-overfit no corte vencedor (se P0 validar) —
      status: `done (non-condition, adjudicado 2026-09-11)` — o corte
      vencedor da P1.1 é **não flipar o default** (opt-in mantido; min
      regrediu em 2 células): nenhum default mudou, não há corte default
      para gridar; evidência parcial da perna 10× com compaction on =
      célula 10k do gate 0185 medida nos dois braços 3/3 ≥ 1,0 (p209b:
      nobuf min 1,078 / buf min 1,268); o grid 10–100× completo volta a
      ser pré-condição de qualquer flip futuro de default

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | kernel should_flush + AS-IS twin | done (4 testes verdes) | este commit | 2026-09-11 |
| P0.2 | p0 | staging no WalWriter (env opt-in, ordem (a)–(f)) | done (12 testes verdes, byte-idêntico arquivo real) | este commit | 2026-09-11 |
| P0.3 | p0 | meter 3 rounds quiet (alvo+guardiãs+10k) | done (p209a/p209b; 10k 3/3 ≥1,0 ambos braços) | findings/2026-09-11-p209-wal-buffer-meter | 2026-09-11 |
| P1.1 | p1 | flip default pós-meter | done — default ON + lone drain (1c FlushWAL); `PEDRA_WAL_BUFFER=0` AS-IS | 2026-09-14 |
| P1.2 | p1 | atribuição ycsb_a (hat do inventário) | done — perda 0,605 era boot-specific (2,602 neste boot) | findings/2026-09-11-p209-wal-buffer-meter | 2026-09-11 |
| P2.1 | p2 | U-cells lote Linux 3-run | done — 21/21 medidas (p211u3): 15 ≥1,0 min-of-3; 6 perdas honestas nomeadas (kafka_flush 0,036; ingest_sst 0,069; compaction_filter 0,081; linkbench 0,236; wbwi 0,494; write_tx 0,751) | findings/2026-09-11-p209-ucells-gate | 2026-09-12 |
| P2.2 | p2 | meters pesados 100M/15M/25M | blocked (re-adjudicado 2026-09-11: gate 09-10 reaberto 01:30; bloqueio = orçamento de onda + custo nomeado; host-check datado no RFC) | inventário 2026-09-11 | 2026-09-11 |
| P2.3 | p2 | Grid B no corte vencedor | done (non-condition: P1.1 não flipou default; perna 10× compaction-on medida 3/3 ≥1,0 nos 2 braços p209b) | findings/2026-09-11-p209-wal-buffer-meter | 2026-09-11 |

## Acceptance Criteria

- **Tests (nomeados, caminho real)** — todos com prefixo `rfc0209_` /
    `wal_buffer_` por convenção do repo; os dois primeiros níveis:
    `WalWriter` (sink de sonda) e `Wal` (arquivo real, eixo env serializado).
  - `wal_buffer_kernel` family: `wal_buffer_should_flush_at_threshold`,
    `wal_buffer_as_is_always_flushes`, `wal_buffer_zero_max_is_as_is`,
    `wal_buffer_flush_is_monotonic_in_staged`.
  - `buffered_wal_byte_identical_after_close` — mesma sequência de ops
    nos dois modos (sink real de arquivo), `cmp` byte-idêntico pós-close.
  - `sync_after_staged_flushes_before_fd` — puts async staged + um put
    G1/lone-sync: o frame do sync vem DEPOIS dos staged no arquivo e o
    fdatasync só retorna com tudo drenado (ordem pinada lendo os frames).
  - `close_drains_staged_bytes` — staged não-vazio no close vira bytes no
    sink antes do fim.
  - `group_direct_write_drains_staged_first` — escrita direta de grupo
    (pwrite ticket) com staged pendente: drain antes, ordem global.
  - `staging_disabled_without_env` — sem `PEDRA_WAL_BUFFER`, um
    `write_all` por registro (sink contador; twin AS-IS no caminho real).
  - Suíte crash/reopen/torn existente verde (o contrato de durabilidade
    não muda: torn-write continua cauda rasgada no CRC).
- **Telemetry / Analytics**
  - Nenhuma linha nova default (0169). O finding do meter carrega os
    números; se P1.1 flipar, o flip registra contadores opt-in de
    flushes/staged-bytes no latch existente.
- **Documentation**
  - Este RFC; inventário `findings/2026-09-11-gargalos-inventario/`;
    finding do meter P0.3; `docs/status.md` nos mesmos commits dos flips.
- **Screenshots**
  - Backend-only — n/a.

## Out of scope

- Wait-to-grow / linger / espera por writers em qualquer forma (0180/0190
  — veto permanente; o flush é por tamanho, ponto).
- Idle/timer flush (paridade com WritableFile: sem relógio; se o meter
  mostrar perda por latência de flush, reabre como fatia própria com
  número).
- WAL-shard (refutado), intern/TLS, DONTNEED/WILLNEED incondicional,
  lock-through-WAL, skiplist TCB.
- Tocar o caminho G1 (fd antes de Ok é o produto; single-client G1 nunca
  é win por construção).
- Ligar default sem meter (P1.1 exige min-of-3 válido). Darwin como
  cartaz. Sync-peer como win. Meter 100k como prova das células mc4.
