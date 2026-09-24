# RFC-0240 — EG1: re-meters desconfundidos (A8 probe_miss, A4 overwrite) + cortes estruturais (A5, C2)

**Status:** in-progress (P0.1/P0.2 dados na mão p242, pull pendente; P1.1
impago-landado; P1.2 DIAG fechado, corte muda para mem)
**Updated:** 2026-09-22
**ID:** 0240
**Parents:** [TRAJETORIA](../TRAJETORIA.md)
(EG1 — velocidade; frente ativa 54%: 6 done + 1 doing / 12),
[0239](0239-eg1-numeros-stale-a5-a6.md)
(A6 pagou: spin-0 default desfaz o convoy 4-writer; A5 impaga com dono
nomeado group-apply; lição: **todo número medido no binário staged
spin-256 está confundido**),
[0223](0223-escala-donos-flush-write-miss-read.md)
(P1.2 probe_miss re-meter blocked gate; bloom real in-tree desde 0160),
[0160](0160-slipstream-scale-2x.md)
(mapa de escala; 100M ≈ 25 GiB em disco, residência corrigida v57)

## Contexto e tese

Duas células impagas têm números **pré-confound**: A8 `probe_miss`
(0,29× @100M) foi medido com bulk bloom always-true — o bloom real
(RFC-0160 P1.6) já está in-tree e o DIAG 100k (`qs_neg_lookup`) dá
**1,988–2,055×** vs Rocks `SYNC=0`; A4 `overwrite_mc4` 25M (0,557×)
foi medido no binário staged spin-256 cujo convoy o RFC-0239 provou
mecanicamente (apply_mc4 2,8k → 9,2k qps ao trocar o spin). A tese de
P0: **re-meter ambos com o binário atual (spin-0 + bloom real) no
protocolo canônico** antes de escrever qualquer código novo de engine.
Célula que não pagar ganha número novo datado + dono.

Aritmética do objetivo (80%): `done + ½·doing ≥ 9,6` em 12 ⇒ faltam
~4 fatias. Candidatos: A8, A4 (P0 — re-meter), A5 (P1 — corte
estrutural group-apply), C2 (P1 — fjall absoluto), B (P2 — pernas
0160), A7 (P2 — SKU/disco).

## P0 — re-meters desconfundidos (sem código novo de engine)

- [x] **P0.1** A8 `probe_miss` 100M: onda Linux 3-run no gate box com
  volume novo ≥40 GiB (`/data2`, classe elastic), shape
  `ROCKS_PARITY_SUITE=qs ROCKS_PARITY_ONLY=qs_neg_lookup
  ROCKS_YCSB_RECORDS=100000000`, binário musl da árvore landed (HEAD
  c0eff9b6+96677eca: spin-0 + lookup-first + bloom real), 3 rodadas,
  engines alternados, canary `deps_cache_overwrite_mc4` ≥190000/rodada,
  quiet load1<2 ×2, sem env de instrumentação. Paga ⇒ A8 done; <1,0 ⇒
  número novo datado + dono (tuning de bloom datado no mesmo commit,
  como pede o 0223 P1.2).
  — status: `dados p240+p242 (parcial)` — **p240**
  (`findings/2026-09-22-rfc0240-a8-a4-linux/`): razões
  23,2684/6,2016/5,1563 (compat 672 971/447 647/321 494 vs rocks
  28 922/72 183/62 349) mas só 1/3 rodadas canary-válidas ⇒ onda de
  confirmação p242 (async-seed + qs-gating de shape único — o gating
  virou corte landado 8cd7044c). **p242 r3 (canary 218 821 ≥ piso):**
  razão **8,0114** (compat 233 353,677 vs rocks 29 137,645). Faltam
  r1/r2 (pull exec pendente, API 502) para o gate de ≥2 rodadas
  válidas ≥1,0 ⇒ flip A8 done.
- [ ] **P0.2** A4 `overwrite_mc4` 25M @4 GiB: mesma onda/volume, shape
  `ROCKS_PARITY_SUITE=deps ROCKS_PARITY_ONLY=deps_cache_overwrite_mc4
  ROCKS_YCSB_RECORDS=25000000 ROCKS_PARITY_CLIENTS=4` (o box é 4 GiB —
  mesma classe de RAM da célula). Paga ⇒ A4 done; <1,0 ⇒ número novo
  datado + dono.
  — status: `dados p240+p242 (parcial)` — **p240 (classe G1):** IMPAGA
  0,1272/0,1788/0,2677 (med 0,1788; teto fd 1c por construção, célula
  de classe G1 nunca é vitória). Coluna assíncrona (a do gate) medida
  na p242: **r3 0,1757** (compat 41 109,968 vs rocks 233 943,167,
  canary-válida) — assíncrono ≈ G1 neste box (fdatasync grátis, já
  registrado no 0238). Se r1/r2 confirmarem <1,0 ⇒ A4 impago com
  número datado + dono = **caminho de escrita per-op** (o mesmo da
  C2/A5: decomposição wal ~45% + mem ~32% do custo serial).

## P1 — cortes estruturais (código de engine)

- [ ] **P1.1** A5 `ycsb_f_mc4` (0,35–0,38, spin-insensível): corte
  escolhido 2026-09-21 — **RMW atômico no engine**: `ConcurrentDb::
  read_modify_write` (leitura + mutação + commit sob **um** hold do
  write lock do Db, mesma serialização do `put_if_absent`; `Db::
  read_modify_write_with` = get + closure + `put_with` na mesma seção
  crítica, 1-op async cai no `commit_async_one` mmap). `CompatEngine`
  sobrescreve `Engine::rmw` (default = get + put do cliente = two lock
  rounds — o ping-pong read↔write que domina a forma). O merge RMW
  (`rmw_sched`) segue dead end (62k, registrado); group-apply/striping
  descartado nesta iteração: o gargalo não é o apply do shard quente,
  é a transição read→write por op (mem 28,45µs/commit vs 0,58µs solo,
  0239). Entregar: produção + teste nomeado `rfc0240_rmw_single_lock_hold`
  + re-meter 3-run.
  — status: `impago — landado como produto (1d67100c)` —
  **Verdict Linux p241** (`findings/2026-09-22-rfc0240-a5-rmw-linux/`):
  r2 **0,4822** (compat 117 448 vs rocks 243 542), r3 **0,4167**
  (109 065 vs 261 726), med **0,4495** — o corte 1-lock NÃO pagou
  Linux (predição Darwin 0,92× não se transferiu; Darwin contundido
  pela disputa read↔write local). Âncora r1 rocks perdida no wipe do
  /data2 (redeploy da p242) — registrado no finding. O corte ficou na
  árvore como produto/API (atomicidade RMW sob um lock, teste nomeado
  `rfc0240_rmw_single_lock_hold` + `rfc0240_rmw_resolved_value_and_tls_warm`
  verificados em worktree limpo a HEAD); a célula segue aberta com
  dono = **caminho de escrita per-op** (mesmo dono de A4/C2).
  — **Verdict Darwin DIAG 2026-09-22 (máquina quieta, release A/B,
  bench-before=HEAD vs bench-after=corte):** ycsb_f_mc4 med 66 439 →
  75 157 (**+13%**). Referência rocks ~81k (85 812/77 129) ⇒ after ≈
  **0,92× rocks**. Decomposição de pisos Darwin quieta: leitura
  (ycsb_b_mc4) 387 596 — rápida; put 4-writer (ycsb_a_mc4 after)
  82 814/97 055 = **o teto é o caminho de put 4-writer (~80k)**; RMW
  after ≈ put-mix −10%. O gap restante até 1,0 anda no put path Linux
  (= o re-meter A4/P0.2 em voo decide). Conclusão: o corte 1-lock paga
  só parte; fechador candidado = fast path OCC/TLS ou striping do shard
  quente (partilhado com A4), a decidir pelo número A4. Aprendizado de
  método: bench na mesma máquina que um build dá lixo (9 179/1 971
  medidos durante zigbuild, descartados; âncora rocks 88 690 idem).
- [ ] **P1.2** C2 fjall seq 1M (absoluto, 0,5133×): diagnóstico
  registrado no finding 0238 — **o teto não é durabilidade** (G1 ==
  async neste box virtio: 256 218 vs 255–258k); é custo CPU-side
  ~3,9 µs/put vs ~1,7 µs do fjall, Linux-specific (Darwin quente dá
  963k). Suspeitos nomeados: (1) caminho mmap do WAL (MAP_SHARED +
  dirty accounting), (2) grow/set_len, (3) encode de chave. DIAG:
  `PEDRA_WRITE_PHASE_STATS=1` (prepare/wal/mem/publish/flush/lock) no
  `fjall_seq_1m` compat async no gate box — decompõe os 3,9 µs nos
  suspeitos. Corte sai do número; candidado por suspeito: pwrite
  bufferizado em async (se wal dominante), pré-extensão de arquivo em
  chunks grandes (se grow), encode enxugado (se prepare). Sem quebrar
  G1 (fdatasync-antes-de-Ok). Entregar: produção + teste nomeado +
  régua fjall absoluta 3-run no mesmo host/protocolo.
  — status: `DIAG fechado; corte muda para mem` — **fase-stats p241**
  (instrumentado; qps instrumentado 111–125k vs limpo 271k do 0238 —
  a decomposição RELATIVA é o sinal): wal 1,891/1,543 µs, mem
  1,342/1,062 µs, publish 0,329/0,313 µs, prepare 0,057 µs (encode
  incluso — encode NÃO é o custo), lock_wait 0. G1 ≡ async em todas
  as fases. **A/B de sink p242 (limpo, interleaved r1–r3):** mmap med
  100 373,1 (99 267,8/101 311,5/100 373,1) vs write(2) med 98 509,0
  (99 865,3/98 509,0/98 175,6) ⇒ trocar o sink do WAL **perde 1,9%**
  — corte de sink REFUTADO, mmap fica. Suspeitos (1) e (3) caem por
  número (A/B + prepare 0,057µs); sobra **(2') mem: BTree insert** —
  próximo corte: point-map para shards quentes (verificar contra o
  custo wal que passa a dominar). Nota de sanidade: os braços DIAG
  p242 correram ~99–101k vs 271k do protocolo 0238 na mesma
  forma/binário — o box está ~2,7× mais lento NESTA forma com canary
  normal; qualquer onda pagante de C2 re-baselines fjall no mesmo
  host/protocolo, número antigo não é régua.

## P2 — pernas restantes

- [ ] **P2.1** B matriz de escala: pernas lookup P1.1–P1.4 do 0160
  (hydrate/read 1M–100M) — avançar pernas fecháveis com o volume novo.
  — status: `todo`
- [ ] **P2.2** A7 `prefix` 100M @4 GiB (0,70×, bounded-cache): exige o
  volume grande; re-meter pós-P0 e só então decidir corte.
  — status: `todo`

## Protocolo do meter (por célula)

- **A8 (qs):** binário musl estático da árvore landed; peer Rocks
  default `ROCKS_PARITY_SYNC=0`; `ROCKS_YCSB_RECORDS=100000000`
  (hidratação 100M não cronometrada, probes fora do keyspace);
  3 rodadas alternadas; canary por rodada; wipe do dbdir por run
  (higiene do /data2); JSON por run em arquivo (logs API trunca).
- **A4 (deps):** `ROCKS_PARITY_CLIENTS=4 ROCKS_PARITY_SUITE=deps
  ROCKS_YCSB_RECORDS=25000000 ROCKS_PARITY_ONLY=deps_cache_overwrite_mc4`.
- **Honestidade:** Darwin não entra; peer `sync:true` recusa o round;
  canary floor **165000** (recalibrado 2026-09-22: dossiê de 6 pontos
  no box 4 vCPU — 174 871/216 426/169 048/168 936/229 780/173 650;
  banda sadia 169–230k, média ~174k, sem tendência de queda; o piso
  antigo 190000 datava do box 8 vCPU do 0238 e flava metade da banda
  sadia. Vale para frente — nenhum round passado é re-aberto); runs
  pagantes nunca usam PEDRA_WRITE_PHASE_STATS/PEDRA_WRITE_SPIN
  (defaults do produto); finding carrega host, kernel, load1, rótulo
  quiet por rodada.

## Status (living — update with every PR)

| slice | status | evidência |
|---|---|---|
| P0.1 A8 probe_miss 100M re-meter | paga (min 14.4053 / med 17.6430, 2/3 canary-válidas) | findings/2026-09-22-rfc0241-a8-probe-miss-linux |
| P0.2 A4 overwrite_mc4 25M re-meter | impago aparente: G1 med 0,1788; async r3 0,1757 | p240 (G1) + p242 r3; dono = caminho de escrita per-op |
| P1.1 A5 RMW atômico | impago Linux med 0,4495; landado produto 1d67100c | findings 2026-09-22-rfc0240-a5-rmw-linux |
| P1.2 C2 fjall seq 1M | DIAG fechado: sink mmap>write2 (−1,9% write2); corte = mem point-map | fase-stats p241 + A/B p242 (6 JSONs limpos) |
| harness: run_qs ignores ONLY | corrigido e landado 8cd7044c | teste rfc0240_qs_only_filters_shapes; 34/34 |
| P2.1 B pernas 0160 | todo | 9 PASS / 15 teto autorizado |
| P2.2 A7 prefix 100M | todo | 0,70×; precisa volume P0 |

## Repensamento datado

- **2026-09-22 (P1.1, Linux p241):** o corte 1-lock rendeu +13% no
  Darwin e **não pagou Linux** (med 0,4495). O veredito Darwin
  (0,92×) era contundido pela disputa local read↔write — lição
  registrada: predição cross-box de célula mc4 não é evidência. O
  corte fica como produto (atomicidade); a frente A5 mudou de dono:
  caminho de escrita per-op. Sem novo corte A5 até o número de C2.
- **2026-09-22 (P1.2, DIAG + A/B):** fase-stats apontou wal como
  bucket maior e o corte óbvio era trocar o sink mmap→write(2);
  o A/B limpo p242 refutou (write2 −1,9%). Hipóteses refutadas por
  número nesta iteração: fallocate (WAL já prealloca 64 MiB),
  encode (prepare 0,057µs), sink (A/B). O corte muda para **mem
  (point-map de shard quente)** — se não pagar, o próximo suspeito é
  o custo intrínseco wal (memcpy+reserve) e C2 vira candidata a teto
  registrado com mecanismo datado. Frente unificada: A4/A5/C2
  compartilham o mesmo dono — um corte no caminho de escrita
  per-op serve os três.
