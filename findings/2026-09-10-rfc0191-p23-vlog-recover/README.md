# RFC-0191 P2.3 passo 20 — `vlog_recover` graduado a atom

Data: 2026-09-10 · Fire 778

## O que subiu

`vlog_recover_action` (vlog_gc_kernel.rs, par `vlog_recover`,
F51/G-swing, handler `open_with_env`) saiu de `close` para `atom`
com um ∀ iff registrado em `formal/aeneas/lean/VlogGc.lean`:

```lean
theorem vlog_recover_action_refuse_open_iff_wants_large_use_new_and_nothing_on_disk :
    ∀ (blob_active wants_large primary_exists use_new new_exists : Bool),
      (vlog_recover_action blob_active wants_large primary_exists use_new new_exists
          = ok VlogRecoverAction.RefuseOpen)
        ↔ (blob_active = false ∧ wants_large = true ∧ primary_exists = false
            ∧ use_new = true ∧ new_exists = false)
```

0 sorry, `lake build VlogGc` verde na primeira build. Planta (dente)
já verde: `vlog_recover_action_on_live_swing_is_not_ok`.

## Por que essa statement

F51 é o único caso da decisão de 5 Bools em que o kernel recusa em vez
de inventar estado: MANIFEST commitou o swing (`use_new`) e nenhum dos
dois arquivos existe. O iff completa o contra-positivo inteiro — nos
outros 31 pontos do domínio o kernel nunca recusa (abre blob/new/
primary, cria primary vazio quando não havia swing, ou roda sem vlog).

## Prova

Domínio finito (2^5 = 32): `cases` nos 5 Bools encadeados com `<;>` e
um único `simp` fecha tudo — com literais, os ites e o `bind` do
`swung_and_staged` reduzem por defeq/simp; cada goal vira `True ↔ True`
(1 caso) ou `False ↔ False` (31 casos).

## Contas (mesma commit)

- `cap_data_fate` 111 → 110; `floor_atom` 20 → 21; `floor_extract` 259 → 258
- `close_proofs.tsv`: linha `atom catalog:vlog_recover …`
- gates `check_depth_floor.py` + `check_product_floor.py` GREEN
  (`data_fate=110<=110`)

## Estado da fila (após este passo)

Faltam 10 descidas para cap ≤ 100; fila viva = 8 → melhor caso cap 102
(déficit 2 confirmado; plano de cobertura no diário Fire 775).
