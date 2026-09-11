# RFC-0200 P1.1 — saída de merge alcançável (base vazia) + corolário

Segunda fatia do RFC-0200. O 0198 P1.3 deixou `merge_chain` como lista
de passos com premissa estrutural; aqui a cadeia ganha a forma de
PRODUÇÃO: a saída nasce vazia e cresce um passo por emissão.

## O que mudou (`formal/aeneas/lean/Merge.lean`)

- `merge_output_reach` — família indutiva Nat-indexada: base `empty`
  (saída vazia, k = 0) e passo `emit` (um `MergeStep` newest-first é
  anexado AO FIM da saída — ordem de emissão).
- `merge_output_reach_chain` — PONTE produção→cadeia: uma saída
  alcançável, lida de trás pra frente, É uma cadeia `merge_chain`
  (o `cons` da cadeia é a emissão mais recente). Prova: indução +
  `List.reverse_append`.
- `merge_output_reach_preserves_inv_lsm` — COROLÁRIO: toda saída
  produzida a partir da vazia (todo topo newest-first) contém apenas
  versões genuinamente live nas que o filtro respondeu live. Composição
  pura: ponte + `merge_chain_preserves_inv_lsm` (0198 P1.3) +
  `List.mem_reverse`. Nada re-provado.

## Verificação (mesmo commit)

- `lake build Merge` verde na primeira tentativa (1701 jobs);
  `grep sorry` = 0. Fatia de lema, sem mudança de registro; gates
  inalterados (depth/product/ledger GREEN no commit anterior, nenhum
  arquivo de registro tocado aqui).
