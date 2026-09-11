# RFC-0191 P2.3 passo 21 — `ae_entry` graduado a atom

Data: 2026-09-10 · Fire 779

## O que subiu

`ae_entry_action` (pedradb-raft/ae_kernel.rs, par `ae_entry`, F16,
handlers `handle_append_entries`/`rpc_append_entries`, live caller
`on_append_entries` no store) saiu de `close` para `atom` com um ∀ iff
registrado em `formal/aeneas/lean/Ae.lean`:

```lean
theorem ae_entry_action_truncate_and_install_iff_conflict_above_commit :
    ∀ (entry_index : U64) (entry_term : U64) (existing_term : Option U64)
      (commit_index : U64) (last_log_index : U64),
      (ae_entry_action entry_index entry_term existing_term commit_index last_log_index
          = ok AeEntryAction.TruncateAndInstall)
        ↔ (∃ t, existing_term = some t ∧ ¬(t = entry_term)
            ∧ ¬(entry_index <= commit_index))
```

0 sorry, `lake build Ae` verde na primeira build. Planta (dente) já
verde: `ae_entry_action_on_live_queued_is_not_ok`
(pedradb-store/three_teeth_queued.rs).

## Prova

- `cases existing_term`; branch `none` nunca produz TruncateAndInstall:
  o corpo tem `lift (saturating_add last 1#u64)` — `lift` é
  `def lift x := ok x` em Aeneas, então `have hl : lift X = ok X := rfl`
  + `rw [hl] at h` + `simp at h` (bind_ok) expõe o ite; `split at h`
  fecha os dois ramos por construtores distintos.
- Branch `some t`: `simp at h` reduz o match; duplo `split at h`
  entrega exatamente `hne : ¬(t = entry_term)` e
  `hgt : ¬(entry_index <= commit_index)` como testemunhas do ∃.
- Backward: `Option.some.injEq` + `subst` + `simp [hne, hgt]`.

## Contas (mesma commit)

- `cap_data_fate` 110 → 109; `floor_atom` 21 → 22; `floor_extract` 258 → 257
- `close_proofs.tsv`: linha `atom catalog:ae_entry …`
- gates `check_depth_floor.py` + `check_product_floor.py` GREEN
  (`data_fate=109<=109`)

## Estado da fila (após este passo)

Faltam 9 descidas para cap ≤ 100; fila viva = 7 (pin_gc, leveling,
cf_encode_effective, compact_rewrites_sst_cf, decode_cf_key,
encode_cf_key, infer_sst_cf) → melhor caso cap 102 (déficit 2
confirmado; plano de cobertura no diário Fire 775).
