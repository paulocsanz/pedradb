# RFC-0198 P1.3 — Inv-LSM corolário indutivo: cadeia de k merges

Terceira fatia indutiva do RFC-0198 (a última das de arquivo limpo).
Fecha a frase "a cadeia de k merges preserva
newest-first-never-non-live": o lema um-passo registrado cobria um topo
de heap; agora a garantia escala para a cadeia inteira por indução.
Peer de referência: nenhum (fatia formal, sem bench).

## O que mudou (`formal/aeneas/lean/Merge.lean`)

- `MergeStep` — estrutura de um passo da cadeia: o par de idades no
  topo do heap (`newer`/`older`, newest primeiro no empate), o kind da
  versão que sobe e o bool de range cobrindo a chave.
- `merge_step_newest_first` — premissa estrutural do passo: no empate
  de chaves o probe 0164 (`first_probe_on_equal_lo`) responde o mais
  novo primeiro (o heap é restaurado newest-first a cada saída).
- `merge_step_answers_live` — o filtro do get (`visible_at`) respondeu
  live para a versão que subiu neste passo.
- `merge_chain` — predicado indutivo Nat-indexado: `nil` (base, cadeia
  vazia, k = 0) e `cons` (um topo newest-first + cadeia de k passos →
  cadeia de k+1). A premissa estrutural é por passo, não de um par
  fixo.
- `merge_chain_preserves_inv_lsm` — COROLÁRIO INDUTIVO: numa cadeia de
  k merges em que todo topo permaneceu newest-first, TODO passo cujo
  filtro respondeu live é genuinamente live (Value não escondido por
  range). Indução sobre a cadeia; o caso do passo CITA o lema um-passo
  REGISTRADO `inv_lsm_newest_first_never_non_live` (RFC-0191 P2.2) —
  nada é re-provado.

## Escolha de forma (por que lista e não estado evoluído)

O extrato MergeKernel não carrega um tipo de heap (o `sift_step` é o
kernel de decisão booleano da estrutura); inventar uma função de
próximo-estado seria modelo sem lastro no extrato. A forma honesta na
escala do modelo: a cadeia É a sequência de passos que subiram, a
premissa estrutural (newest-first em todo topo) viaja no construtor, e
o corolário é exatamente a forma seL4 — invariante do sistema por
indução sobre os passos, passo = lema registrado. Base honesta: cadeia
vazia (nenhuma versão subiu — o invariante é condicional por passo).

## Verificação (mesmo commit)

- `lake build Merge` verde na primeira tentativa; `grep sorry` no
  arquivo = 0.
- `bash scripts/lean_extracts.sh --required` verde (61 libs + 5
  compose); warnings de `sorry` do output são da lib Aeneas
  (`Slice.lean`/`StringIter.lean`), não deste arquivo.
- Gates depth/product/ledger GREEN no commit (sem mudança de registro
  — fatia de lema, não de escada; arquivos de registro não tocados).
