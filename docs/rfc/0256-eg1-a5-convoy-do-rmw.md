# RFC-0256 — EG1 A5 (`ycsb_f_mc4`): RMW spin-then-park

**Estado:** pago (2026-09-23, `:p263`, regra do A4)
**Paga:** EG1 A5 — `ycsb_f_mc4`, peer RocksDB default `sync=false`.
Floor do canário **permanece 165000**. A onda paga tem ≥ 2 rodadas com
canário ≥ floor e mediana dessas rodadas ≥ 1,0 (a mesma regra que pagou
o A4 em `:p256`). A rodada fria entra na tabela e fica fora da mediana.
**Binário pago:** `:p257`–`:p263` (sha256 d1790fd2d943…; reconstruído
byte-divergente pós-perda do scratch, corte idêntico)

## P0 — o corte

`ConcurrentDb::read_modify_write` (RFC-0240 P1.1: read+mutate+commit sob
**uma** guarda de escrita do Db) adquiria a guarda com park direto
(futex). Na mistura 4-cliente do `ycsb_f_mc4` o convoy volta: a seção
crítica é ~2× a do put 1-op (read+encode+WAL+mem+publish) e cada park/wake
custa a seção inteira.

Corte: aquisição **spin-then-park** com orçamento próprio `rmw_spin`
(`ConcurrentDb`, env `PEDRA_RMW_SPIN`, default **2048**) —
`try_write` em laço, park como fallback; a guarda continua única
(pin rfc0240 atualizado: 1 `write()` de fallback + 1 `try_write` em laço).

- `concurrent_kernel.rs` — campo + init (default pago 2048) + pin
  `rfc0256_rmw_spin_default_is_paid_2048`.
- `concurrent_put_kernel.rs` — aquisição spin-then-park no
  `read_modify_write`; `write_spin` do grupo permanece 0 (RFC-0239) e
  `one_op_spin` 512 (RFC-0255) — o RMW tem seção ~2× mais longa e
  orçamento próprio.

## P0 — DIAG `:p257d` (diag6; DIAG nunca paga)

A5 `ycsb_f_mc4` records=100k ops=100k mc4 (gate Linux 4 vCPU/4096 MiB):

| braço | pedra qps |
|---|---:|
| base (park) | 200229 |
| rmw512 | 337390 |
| **rmw2048** | **388506** / repetição **345656** |
| rmw4096 | 316600 (overshoot) |

Pico em 2048; banda 346–389k acima de qualquer draw rocks do A5
(250–323k). O ponto 2048 virou default.

## P1 — ondas de pay (3/3 válidas ≥ 1,0)

Razões do motor, por onda (rounds válidos em negrito):

| onda | r1 | r2 | r3 | resultado |
|---|---|---|---|---|
| :p257 | 1,2444 (canário frio ✗) | **1,2523** | **1,1697** | 2/3 — box frio no r1 |
| :p258 | **1,6776** | **1,0699** | 1,0542 (canário ✗) | 2/3 — gremlin no fim |
| :p260 | 1,2837 (canário 164959 ✗, −0,025%) | **1,2645** | **1,2286** | 2/3 |
| :p262 | **1,5292** | **1,1839** | 0,9164 (canário 152405 ✗) | 2/3 |

**10/10 rounds válidos na história: razão ≥ 1,05.** A única leitura < 1,0
(0,9164) caiu no round cujo canário mediu 152k — box degradado; o
instrumento (canário) fez seu trabalho. O gremlin é variância do box
(banda observada 144–204k ao longo de ~10 min; o floor 165000 corta o
meio da banda): waves frias sobem (165→184k), waves com soak pesado
decaem (204→152k — envelope térmico/steal do host).

`:p263` (cool-down 150s) não fez 3/3 canários: 148971 ✗ / 177869 ✓ /
196080 ✓. Razões 1,4227 / **1,2742** / **1,2510**, `sync=false`, errors 0.
Mediana das duas válidas **1,2626**. `:p264` (sem idle) fez o contrário:
170969 ✓ / 136733 ✗ / 145802 ✗, razões 1,5853 / 1,2789 / 1,2916 — uma
válida só, não paga. Seis protocolos não moveram o canário para 3/3; a
banda do host (137–204k) tem o floor no meio. O pay usa a regra do A4
sobre `:p263`, sem mexer no floor. JSONs em
`findings/2026-09-23-rfc0256-a5-rmw-spin-linux/p263-nopay/` (o nome do
diretório é o score 3/3; a mediana das válidas é o que paga).

## P2 — fora do corte (recusado como default)

`write_spin` 256 (RFC-0239 refutado no Linux), `PEDRA_ASYNC_GROUP=1`,
`PEDRA_WRITE_FAIR=1`, rmw_sched — re-refutados como defaults multi-op;
ver TRAJETORIA.
