# run-suffix v3 (WIP, NÃO LANÇADO) — patch preservado 2026-08-23

Estado: implementação completa + 4 testes (395 lib verdes), **regressão real
em ycsb_a isolada (0,837 — old 3.334 k / new 2.790 k, 5 rounds, fresh DB)** e
regressão herdada em cache_overwrite quando a suíte roda inteira (0,258 —
spill one-shot do run de ~32k entradas do deps_raftlog caindo na perna
seguinte; isolada = 0,898 neutra).

Micro (não-arming vs arming): 0,197 → 0,051 µs/op (−74%); probe 20k NOFLUSH
4,13 µs/batch (pre) vs 5,87 (post). A/B raftlog-only sujo: 1,056.

Hipótese aberta para ycsb_a: o seed não-temporizado (1024 puts ascendentes)
arma o run; o primeiro update zipf derrama ~1013 entradas (~200 µs) numa
perna de ~600 µs. Discriminador barato não executado:
`ROCKS_YCSB_RECORDS=64` (seed 64 ≤ RUN_ARM+1 nunca arma → regressão some se
a hipótese vale).

**DISCRIMINADOR EXECUTADO 2026-08-23 (5 rounds × 2, ycsb_a isolada, caixa
suja)**: hipótese **CONFIRMADA**. `records=1024`: old med ~3,10 M / new med
~2,53 M (0,82 — regressão reproduz; new max_ms 116–124 µs em TODOS os
rounds = assinatura do spill do seed; old max 13–29 µs). `records=64`:
regressão SOME (new 3,16–3,24 M tight; old 1,86–3,31 M com ruído de caixa).
Causa = spill one-shot de ~1013 entradas na primeira perna temporizada.

**Consequência para o design (números)**: o custo TOTAL do spill é o que
mata a perna vizinha — amortizar por op (bound B/op) remove o pico de
max_ms mas soma o mesmo ~0,19 µs/entry na média da perna seguinte
(cache_overwrite na suíte: ~28k entradas do raftlog ⇒ ~5,3 ms somados numa
perna de 1,5 ms ⇒ ~0,25× spike OU diluído — qps ruim dos dois jeitos).
RUN_ARM maior (4096) não resolve a suíte (o spill do raftlog 32k só fica
maior). **O design certo é multi-run lazy**: op fora-de-ordem NÃO derrama —
abre run novo (lista de runs delimitada, ex. ≤8; derramar o mais velho ao
exceder); leitor faz merge map + idx + N runs (busca binária por run,
todas ascendentes-disjuntas). ycsb_a: seed vira 1 run parado + steady-state
indexado — 2 runs, custo de leitura desprezível. Implementação: generalizar
`SuffixRange`/`MemInternalIdx` (3-way → (2+N)-way) com lista de fronteiras
de run no `tail`.

Patch: `run-suffix-v3-uncommitted.patch` (contra d343044).

**Decisão atual: revertido da árvore de trabalho** (sessão paralela ativa;
stall-extent APFS do WAL priorizado — ver `findings/2026-08-22-rearm7/`).
Retomar só com design que não pague spill one-shot em perna vizinha
(lazy multi-run reads / orçamento de spill / arm por tamanho de run —
RUN_ARM≈4096 ainda derrama ~28k na perna seguinte da suíte).
