# RFC-0189 P2.2 — `mins` híbrido: condição NÃO atingida — sem corte

**Data:** 2026-09-10
**Veredito:** não-disparo registrado com números; nenhum código aterrado (por desenho:
o slice é condicional à atribuição).

## A condição (RFC-0189 P2.2)

"`mins` híbrido (BTreeMap permanece) só se P0.1 mostrar explosão na janela lenta; gate
anti-regressão `ycsb_c`."

## O que o P0.1 mediu (`2026-09-10-rfc0189-p01-atribuicao`, Linux quieto p149b)

- `mins` = **0.58 µs/op** na perna quieta (qps 222.125), estável na banda
  **0.58–0.66 µs/op** entre janela boa e janela lenta — sem explosão.
- Para comparação, as fatias que o P0.1 indiciou de verdade: guard = **2.23 µs/op**
  (→ P2.1 disparou, execução no RFC-0190 P0.2) e wr/syscall = 0.61–0.98 µs/op
  (→ P1.2 write off-lock).

## Conclusão

O custo de `mins` (range-min do BTree para janelas de contagem) é pequeno e FLAT entre
as janelas — não é o que decide o min-of-3. Um híbrido (BTreeMap + estrutura extra)
adicionaria estado e um gate `ycsb_c` para cortar uma fatia que a medição já absolveu.
Corte rejeitado pela própria atribuição que o condicionava; `memtable.rs` intocado por
este slice.

Reabrir somente se uma futura atribuição mostrar `mins` > ~1.5 µs/op OU explosão
>janela boa × 2 na janela lenta.
