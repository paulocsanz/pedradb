# RFC-0239 — EG1: A5 `ycsb_f_mc4` / A6 `apply_mc4` Linux + C1 ignorado por pedido do operador

**Status:** A6 PAGA (P0.4 done — spin-0 default; P1.1 done — 1,1852/1,2315/1,2983;
P0.1 A5 measured-unpaid c/ dono; P0.2 done; P0.3 landed; P2 todo) — **EG1 54%**
**Updated:** 2026-09-21 (onda `wave-p239w` + sweep `p239s` + onda `p239g` **pagando A6**)
**ID:** 0239
**Parents:** [TRAJETORIA](../TRAJETORIA.md)
(EG1 — velocidade; gate sequencial: EG1 é a frente ativa),
[0238](0238-eg1-pagaveis-ycsb-b-mc4-ladder-fjall.md)
(A9 pagou com 3-run quiet no gate box 4 vCPU; canary floor calibrado
190000 com desvio documentado; C1/C2 mediram unpaid),
[0233](0233-ganhar-sempre-rocks-fjall-default.md)
(P1.4, 2026-09-16: `rmw_sched` default OFF + `write_spin` default 256 +
group mmap `write_wal` — Darwin `ycsb_f_mc4` min-of-3 0.732→1.022),
[0193](0193-write-off-lock-pwrite-ticket.md)
(P0.5 adjudicou `apply_mc4` same-class **1.0859× min-of-3** Linux quiet,
2026-09-11, `findings/2026-09-11-p201r2-mc4/`)
**Peer Rocks:** `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`), JSON
`sync:false` do peer no finding. Darwin = DIAG. Linux 3-run quiet = cartaz.

> **Tese:** duas células EG1 têm números Linux **stale** — medidos antes
> de cortes de engine já commitados que as movem — e uma fatia foi
> retirada da escada pelo operador. (1) `ycsb_f_mc4` (A5) mede
> 0.295–0.634× em 2026-09-11 (p201r2) e "run2 0.766×" numa era anterior;
> os cortes do RFC-0233 P1.4 (2026-09-16) mudaram exatamente o dono dessa
> shape (merge RMW obsoleto + park contendido) e lifting Darwin
> 0.732→1.022 — o número Linux simplesmente nunca foi re-medido. (2)
> `apply_mc4` same-class (A6) foi adjudicada **paga** (1.0859× min-of-3,
> RFC-0193 P0.5) e a TRAJETORIA registrou `todo` por ler a tabela rank
> stale — precisa de re-confirmação com o binário atual e o flip. (3) O
> operador pediu "ignorar C1 no endgoal" (2026-09-21) — a escada EG1 cai
> de 13 para 12 fatias. Com A5 paga: 6 done + 1 doing de 12 = **54%**
> (linha dos 50% do goal); com A6 confirmada: 7,5/12 = 62%.
>
> **Diagnóstico (2026-09-21, probes p239 wave/ab/p4 no gate box):** o
> freshly-measured A5 0.29–0.30× **não é stale** — é contenção real na
> seção crítica do insert do memtable. Evidência: `ycsb_f` **1 client =
> 304k QPS** (3,3µs/op, acima do Rocks 4c) vs 66–81k em 4 clients;
> `apply_mc4` (um writer por vez) paga **0,58µs/put** pelo mesmo trabalho;
> `PEDRA_WRITE_PHASE_STATS=1` mostra `mem` 28,45µs/commit vs `wal`
> lock_wait 0,14µs (lock sem alocação = barato). O serializador é o
> `Bytes::copy_from_slice(pfx)` **por put** do `entry()` do shard — a
> única alocação steady-state do insert (point map pré-reservado; short
> BTree in-place; ord-btree inerte em janela pure-write) — dentro do
> write lock do memtable, em musl mallocng cross-thread. A bimodalidade
> da A6 (modo lento 2,9–3,4k) é o mesmo convoy: o clock por fase do
> phase-stats quebra o alinhamento dos writers; toda run rápida desde a
> onda teve phase-stats ON. Runs de pagamento não podem depender de
> pacing por instrumentação — o corte real é eliminar a alocação.

## Rank (impacto × pagável-agora)

| # | fatia TRAJETORIA | por que agora |
|---|---|---|
| 1 | A5 `ycsb_f_mc4` | stale pós-P1.4; mesmo protocolo/escala do A9 que pagou no mesmo box; única barreira dos 50% |
| 2 | P0.2 C1-ignore | pedido do operador; muda denominador e % no mesmo commit |
| 3 | A6 `apply_mc4` | prior pago 1.0859× (0193 P0.5) não registrado na escada; re-confirmar = flip |
| — | A4/A7/A8/B/C2 | caixa/dataset-pinned (25M–100M) ou engine-deep (C2 ext4/jbd2) — fora deste RFC |

## Delivery slices (mandatory)

### P0 — as fatias pagáveis

- [ ] **P0.1** A5: `ycsb_f_mc4` Linux 3-run fresh — **measured-unpaid
  2026-09-21** (`findings/2026-09-21-rfc0239-a5-a6-convoy-unpaid/`,
  onda `wave-p239w`): 0,3711 / 0,3436 / 0,3149 (r1 invalidado por
  canary 188 299 < 190000; r2/r3 válidos, quiet, peer `sync:false`
  15/15, errors 0). Dono do número: **mutação serializada do shard
  compartilhado** — 1c = 304k (probe4) e 4c ≈ 304k/4; os 4 clients
  escrevem no mesmo prefixo `ycsb/` → mesma `TailShard` → ping-pong de
  cacheline + handoff do write lock do memtable. Próximo dono nomeado:
  **group-apply** (os N writers entrarem na seção crítica em grupo,
  leader aplica os batches de todos — espelha a write queue do Rocks)
  ou striping do shard quente.
  — status: `measured-unpaid, dono nomeado`
- [ ] **P0.2** C1 sai da escada EG1 por **pedido do operador**
  (2026-09-21, "é pra ignorarmos c1 no endgoal"): denominador 13→12,
  histórico do endgoal registra a mudança com data, Progresso refeito
  pela fórmula no mesmo commit. C2 segue na escada.
  — status: `done-no-commit (land desta iteração)`

- [ ] **P0.3** Engine cut: `tail_append` **lookup-first** — hot prefix
  acha o shard via `tail_idx.get_mut(pfx)` (`Borrow<[u8]>`, zero
  alocação); o `entry(Bytes::copy_from_slice(pfx))` fica só no miss
  (prefixo novo). Corta a única alocação per-put da seção crítica do
  memtable. Testes nomeados
  `rfc0239_tail_append_lookup_first_shard_reuse` (comportamental: um
  shard por prefixo quente, fallback `entry`, sem contaminação cruzada) +
  `rfc0239_tail_append_lookup_first_source_pin` (fonte: `get_mut(pfx)`
  precede `entry`, a alocação incondicional não volta). **Landed como
  melhoria estrita**, mas **insuficiente sozinha para A5**: a banda
  74–82k não mudou (ver P0.1). Never-regression: suite completa
  staged+cut = 1060 passou / 36 falhou / 4 ignored vs controle
  staged-only = 49 falhas únicas pré-existentes (banda Darwin conhecida,
  memória do workspace); a única delta candidata
  (`rfc0201_auto_async_merge_oversubscribed_herd`) passou 3/3 no re-run
  — flaky de timing, não regressão.
  — status: `landed`
- [ ] **P0.4** Engine default: `PEDRA_WRITE_SPIN` **256→0** no código
  (product default; env continua override). Evidência (sweep `p239s`,
  gate box, **sem** phase-stats): `deps_apply_batch_mc4` compat
  2767–2858 qps em spin-256 vs **9224 / 9988 em spin-0** (Rocks default
  5930–6790; ratio ~1,42–1,47×); `ycsb_f_mc4` é spin-insensível
  (74–82k em 0/256/8192); `PEDRA_RMW_SCHED=1` também resgata
  (8927/9643) mas spin-0 é mais forte e não reintroduz o merge RMW
  (62k em ycsb_f, dead end registrado). Mecanismo: o loop de spin
  queima as mesmas cachelines que a seção crítica do memtable muta — o
  park imediato do futex (wake justo) desfaz o convoy. O default-256
  vinha do RFC-0233 P1.4 (lift Darwin-only, DIAG, nunca cartaz); o
  custo Darwin registrado lá (339k parked vs 466–495k) permanece DIAG.
  Teste `rfc0239_write_spin_default_is_park_immediately` (green no
  worktree HEAD+mine, 3/3 rfc0239). Onda `p239g` **PAGOU A6**
  (re-rodada nativa 2026-09-21, ver P1.1): canary 227919/212510/215036
  3/3 ≥190000, A6 1,2983/1,2315/1,1852 —
  `findings/2026-09-21-rfc0239-a6-apply-mc4-spin0-linux/`. Tentativa 1
  da onda descartada pelo floor (canary r1 189215, box frio pós-restart;
  JSONs preservados no box) e incidente de I/O do VM documentado no
  finding.
  — status: `done (2026-09-21)`

### P1 — re-confirmação de célula já adjudicada

- [x] **P1.1** A6: `deps_apply_batch_mc4` same-class Linux 3-run —
  **PAGA na onda `p239g`** (2026-09-21, re-rodada nativa, spin-0
  default do P0.4 + corte P0.3 no binário):
  **1,2983 / 1,2315 / 1,1852** (min 1,1852, med 1,2315), canary
  227919/212510/215036 3/3 ≥190000, peer `sync:false` 15/15, errors 0 —
  `findings/2026-09-21-rfc0239-a6-apply-mc4-spin0-linux/`. Histórico
  da reabertura: o prior pago 1,0859× (RFC-0193 P0.5, 2026-09-11) **não
  reproduzia** no binário atual sem instrumentação — onda `wave-p239w`
  0,4686/0,4075/0,4542; a causa era o default spin-256 staged
  (RFC-0233 P1.4) queimando as cachelines da seção crítica (convoy
  4-writer); spin-0 dissipa e devolve a folga. A5 `ycsb_f_mc4`
  re-numerada na mesma onda: 0,3829/0,3488/0,3583 — spin-insensível,
  dono nomeado segue group-apply/hot-shard striping (P0.1).
  — status: `done (2026-09-21) — A6 done, EG1 54%`

### P2 — engine-deep (fora do escopo 50%)

- [ ] **P2.1** C2 fjall seq 1M (0.5133×; bloqueio ~50% wall em
  jbd2/ext4 do /data do box — ver probes RFC-0238). — status: `todo`
- [ ] **P2.2** A4 `overwrite_mc4` 25M @4 GiB (0.557×; desbloqueia após
  SIGSEGV rollover 0237 P1.3). — status: `todo`

## Protocolo do meter (por célula)

- **A5 (Rocks):** binário estático musl x86_64 do gate box
  (`/mnt/container/rootfs/usr/local/bin/rocks-parity-bench`, build
  2026-09-21 da árvore com todos os cortes). `ROCKS_PARITY_SYNC=0
  ROCKS_PARITY_CLIENTS=4 ROCKS_PARITY_SUITE=ycsb
  ROCKS_YCSB_RECORDS=100000 ROCKS_YCSB_OPS=100000
  ROCKS_YCSB_PAYLOAD=100 ROCKS_YCSB_DIST=zipfian
  ROCKS_PARITY_ONLY=ycsb_f_mc4` (+`PEDRA_PARITY_ASYNC=1` no compat),
  3 rounds, engines alternados, canary por round, wipe do dbdir após
  captura do JSON (higiene do /data de 974 MB).
- **A6 (Rocks):** `ROCKS_PARITY_SYNC=0 ROCKS_PARITY_CLIENTS=4
  ROCKS_PARITY_SUITE=deps ROCKS_YCSB_OPS=2000` (espelha p201r2;
  `run_deps_clients` emite apply+raftlog mc, seeding simétrico).
- **Honestidade:** Darwin não entra; peer `sync:true` recusa o round;
  Rocks colapsado recusa o round (canary floor 190000 calibrado para
  este box — desvio do default absoluto 260000 documentado no 0238);
  o finding carrega host, kernel, load1 e o rótulo quiet por round.

## Status (living — update with every PR)

| slice | status | evidência |
|---|---|---|
| P0.1 A5 `ycsb_f_mc4` 3-run | measured-unpaid, dono nomeado | `findings/2026-09-21-rfc0239-a5-a6-convoy-unpaid/` (onda `wave-p239w`; min válido 0,3149) |
| P0.2 C1-ignore (denominador 12) | done-no-commit | pedido do operador 2026-09-21 |
| P0.3 `tail_append` lookup-first | landed | testes rfc0239 green; banda A5 inalterada (melhoria estrita, não fecha A5) |
| P0.4 `write_spin` default 0 | **done (2026-09-21)** | onda `p239g`: canary 3/3 ≥190000; A6 1,2983/1,2315/1,1852 — `findings/2026-09-21-rfc0239-a6-apply-mc4-spin0-linux/`; teste rfc0239 green |
| P1.1 A6 `apply_mc4` 3-run | **done — paga (2026-09-21)** | min 1,1852 / med 1,2315 (3/3 ≥1.0), mesmo finding; wave-p239w 0,41–0,47× foi o confound spin-256 staged |
| P2.1 C2 seq 1M | todo | `findings/2026-09-21-rfc0238-c1-c2-fjall-seq-linux/` |
| P2.2 A4 overwrite 25M | todo | rank otimizar 0.557× |

## Repensamento datado (2026-09-21, iteração sem fatia na primeira onda)

A onda `wave-p239w` fechou **zero fatias** (A5 impaga; A6 reaberta pior).
O que mudou de crença: (1) o número A5 0,29–0,30× anterior não era
stale — é contenção real, e o corte de alocação (P0.3) era secundário;
(2) **toda adjudicação anterior de knobs estava confundida** pelo
`PEDRA_WRITE_PHASE_STATS` (o clock por fase espaça os writers e quebra
o convoy — "recuperação" pw0/buf0 era o pacing, não o knob); (3) o
convoy tem dono mecânico: spin-256 queima as cachelines da seção
crítica. Ação tomada em vez de parar: sweep sem instrumentação (p239s)
→ spin-0 resgata apply_mc4 com folga → default virou código (P0.4) →
onda pagando `p239g`. **Resultado: `p239g` PAGOU** (P1.1 acima) — a
hipótese do convoy-mecânico se confirmou; o próximo corte estrutural
(group-apply) fica como dono nomeado de A5, não de A6.
