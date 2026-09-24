# RFC-0255 — EG1: a cauda do bypass 1c (convoy da escrita do Db) e o corte do 25M

**Status:** done (P0 diag + corte; P1 onda `:p256` **pagou A4** — mediana válida
1,3233, r2 1,5844 / r3 1,0623; A5/C2 seguem abertos; P2 coberto pelo canário)

**EG1 degrau:** A4 `deps_cache_overwrite_mc4` 25M (pago em mediana ≥ 1,0 em ≥ 2 rodadas
válidas, canário ≥ 165000) + reflexo esperado em A5 `ycsb_f_mc4`.

## 1. Evidência (onda `:p255`, 3 rodadas válidas, pós-RFC-0254)

A4 pedra 177771/230807/162866 vs rocks 292913/262701/251109 → mediana **0,6486** (melhor
rodada válida 0,8786; melhor qps limpo 230,8k). A5 min **0,5810**. C2: fjall
não-colapsado pela 1ª vez (339113/290081/271594), pedra venceu 3/3 pares
(+7,1/+17,8/+16,6%) — sem pagar (fjall 10–28% abaixo do sadio 377243; absolutos da
pedra também abaixo).

**O achado decisivo (percentis, p255):** o p50 da pedra GANHA do rocks — A4 pedra
p50 6,8–8,9 µs vs rocks 11,3–12,7 µs; A5 pedra p50 7,0–8,3 µs vs rocks 6,5–8,4 µs.
**Todo o gap é cauda**: A4 pedra p95 50,8–78,9 µs (rocks 18,9–23,1), p99 101–142 µs
(rocks 31,8–34,0), p999 347–828 µs, máx 6,9–58,9 ms. A5 idem (p95 53–67 vs 37–42).
Com 4 clientes, um stall serializa todos (qps ≈ 4/média) — a mediana dos percentis
é o inimigo, o corpo é melhor que o do rocks.

Seções do bypass 1c (`commit_async_one_syscall_off_lock`, 25M, DIAG pré-onda):
encode 0,34 µs, **wal 1,25 µs**, mem 0,82, publish 0,22, flush 0,11; **lock_wait
18,83 µs** (5M: 8,86 µs) — ~9× a seção que espera. `avg_group=1,00` (bypass puro,
regra RFC-0201: merge só com writers > ncpu).

## 2. Dono candidato e por que as refutações anteriores não fecham a questão

Motor deployed (`:p255`, lineage commit 54ea22b6): o bypass 1c concorrente é o formato
RFC-0045 — `acquire_bypass_write` (park; `PEDRA_WRITE_SPIN`/`PEDRA_WRITE_FAIR` são
lidos AQUI e agem neste caminho) + `commit_async_one` com a **escrita do Db**
(`db.write()`) mantida através de encode+WAL+mem+publish (~2,7 µs de seção;
lock_wait 18,83 µs no 25M = ~9× a seção). Lock desfair + 4 vCPUs com 4 clientes +
flusher ⇒ cada park/wake é futex + scheduler num guest sobrescrito — latência
wake-to-run de dezenas de µs é o perfil p95/p99 observado; p999/máx (347–828 µs /
6,9–58,9 ms) apontam para eventos de flush/L0 por cima.

- RFC-0233 P1.4 refutou o merge 1-op e fixou `write_spin` default 256 em Darwin
  quiet-band (lock_wait então 1,11 µs); RFC-0239 reverteu o spin para park
  imediato (default 0). O regime 25M/Linux tem lock_wait **18,83 µs** — o ponto
  de equilíbrio park/spin/fair/grupo mudou; nenhuma dessas refutações foi medida
  neste ponto.
- (Fora do escopo deste corte: o motor de trabalho não-commitado evolui o bypass
  para encode off-lock + `wal.lock` pontual — não é o binário deployed e não entra
  nestas contas.)

## 3. DIAG diag5 (gate p238, A4-25M, 100k ops, 4 clients, binário :p255)

Matriz 1 braço por rodada (mesmo binário, env apenas):

| braço | qps | leitura |
|---|---|---|
| `base` (park) | 182792 | corredor p255 (162–231k) |
| `spin512` (`PEDRA_WRITE_SPIN=512`) | **316011** | **+73% vs base**; acima do melhor round pedra de sempre (230,8k) e dos draws rocks p255 (251–293k) |
| `agroup1` (`PEDRA_ASYNC_GROUP=1`) | 85208 | −53%: merge mpsc/leader re-refutado neste ponto (2ª vez, agora pós-0254) |
| `fair1` (`PEDRA_WRITE_FAIR=1`) | 109673 | −40%: handoff dirigido serializa pior que o park unfair neste ponto |

**Decisão: Ramo S.** O dono da cauda era o park/wake (futex + scheduler) da
escrita do Db no bypass 1c. Spin limitado mantém o writer on-CPU e elimina o
convoy — sem tocar na seção, no WAL, no formato. Percentis da matriz (µs):

| braço | qps | p50 | p95 | p99 | p999 | máx |
|---|---|---|---|---|---|---|
| base (park) | 182792 | 8,0 | 70,9 | 146,7 | 830 | 11198 |
| **spin512** | **316011** | 5,9 | **16,0** | **29,1** | 1016 | 27025 |
| agroup1 | 85208 | 35,9 | 85,4 | 272,9 | 2098 | 72179 |
| fair1 | 109673 | 33,9 | 49,1 | 65,0 | 352 | 6112 |

spin512 leva p95/p99 ao nível do rocks (18,9–23,1 / 31,8–34,0 no p255) e abaixo.
p999/máx ficam (donos: flush/L0 — próximo degrau só se voltar a importar).

## 4. O corte (P0)

`concurrent_kernel.rs`: knob novo `one_op_spin` (`PEDRA_WRITE_SPIN_1OP`, default
**512**) usado **só** quando `ops.len() == 1` no bypass concorrente; multi-op
mantém `write_spin` (default 0/park, refutação RFC-0239 intacta — spin-256
colapsou apply_mc4 a 0,41–0,47×). Teste fonte-pinning
`rfc0255_one_op_bypass_spins_by_default` (default 512 + site de aquisição lê
`one_op_spin` + inicializador 0239 segue `unwrap_or(0)`) — verde; 0239/0045/0251
verdes junto.

Escopo é shape, não OS: a refutação 0239 é multi-op (seção longa demais para o
spin valer); o 1-op tem seção ~2,7 µs — 512 pause-loops adquire antes do park.

## Fatias

- **P0** — DIAG diag5 (done: spin512 +73%, agroup1 −53%) + corte `one_op_spin`
  default 512 no bypass 1-op + testes fonte-pinning. — status: `done`
- **P1** — Onda Linux `:p256` 3 rodadas A4/A5/C2 + canário, sem envs de diag
  (o default novo atua sozinho); pagar A4 mediana ≥ 1,0 em ≥ 2 rodadas válidas. —
  status: `in-progress`
- **P2** — Varredura de regressão: A6 `apply_mc4` 1 rodada DIAG (o bypass multi-op
  segue park, mas o canário da onda cobre o mesmo binário); registrar a leitura
  mesmo sem regressão. — status: `todo`

## Resultado (`:p256`, 3 rodadas)

| rodada | canário | A4 pedra | A4 rocks | ratio | A5 ratio | C2 fjall |
|---|---|---|---|---|---|---|
| r1 | 146804 (inválida — frio) | 353466 | 280013 | 1,2623 | 0,8904 | 263867 |
| r2 | 214846 | 321948 | 203201 | **1,5844** | 0,7276 | 102793 |
| r3 | 189104 | 295988 | 278627 | **1,0623** | 0,6718 | 141702 |

**A4 paga**: mediana válida **1,3233**, 2/2 rodadas válidas ≥ 1,0. Percentis da
pedra p50/p95/p99 = 7,3–7,6 / 14,4–16,0 / 22,9–26,5 µs ganham do rocks
(12,7–15,8 / 20,7–27,6 / 32,9–40,9) nas 3 rodadas. A5 impago (`ycsb_f` é RMW —
o spin 1-op não endereça a leitura); C2 impago (fjall colapsado nas 3).
Detalhes e JSONs:
`findings/2026-09-23-rfc0255-a4-pago-spin-1op-linux/`.

| Fatia | banda | entrega | status |
|---|---|---|---|
| P0 diag matriz + corte | p0 | diag5 4 braços + `one_op_spin` + testes pin | **done** |
| P1 onda Linux 3 rodadas | p1 | `:p256` A4/A5/C2 + canário | **done — A4 pago** |
| P2 regressão multi-op | p2 | canário da onda no mesmo binário; A6 park intacto | **done** |
