# RFC-0191 P2.3 passo 23 — `pin_gc` graduado a atom

Data: 2026-09-10 · Fire 781

## O que subiu

`gc_oldest_from_pin` (compact_kernel.rs, par `pin_gc`,
RFC-0150 P2b/F20, handler `compact_reclaim`) saiu de `close` para
`atom` com um ∀ iff registrado em `formal/aeneas/lean/Compact.lean`:

```lean
theorem gc_oldest_from_pin_value_iff_pin_or_unpinned_visible_min :
    ∀ (oldest_pin : Option U64) (last_seq : U64) (visible_seq : U64) (v : U64),
      (gc_oldest_from_pin oldest_pin last_seq visible_seq = ok v)
        ↔ (oldest_pin = some v
            ∨ (oldest_pin = none ∧
                core.cmp.Ord.min.trait_default core.cmp.OrdU64 last_seq visible_seq
                  = ok v))
```

0 sorry, `lake build Compact` verde na primeira build. Planta (dente)
verde: `gc_oldest_from_pin_on_live_reclaim_is_not_ok`
(pedradb-sim/three_teeth_plants.rs).

## Por que essa forma

O branch sem pin devolve o `Ord.min` monádico — stating over its
Result mantém o iff honesto sem precisar provar nada sobre o min
(fail/div do min simplesmente não produz valor). Com pin, o valor é
exatamente o pin — a cerca de GC do F20.

## Prova

`unfold gc_oldest_from_pin; cases oldest_pin <;> simp` — branch some:
`ok p = ok v ↔ some p = some v` fecha por injEq dos dois lados;
branch none: `False ∨ (True ∧ X) ↔ X` deixa `X ↔ X`.

## Contas (mesma commit)

- `cap_data_fate` 108 → 107; `floor_atom` 23 → 24; `floor_extract` 256 → 255
- `close_proofs.tsv`: linha `atom catalog:pin_gc …`
- gates GREEN (`data_fate=107<=107`)

## Estado da fila (após este passo)

Faltam 7 descidas para cap ≤ 100; fila viva = 5 (leveling,
compact_rewrites_sst_cf, decode_cf_key, encode_cf_key, infer_sst_cf)
→ melhor caso cap 102 (déficit 2; plano no diário Fire 775).
