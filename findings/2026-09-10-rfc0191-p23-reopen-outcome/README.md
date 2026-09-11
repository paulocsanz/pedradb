# RFC-0191 P2.3 passo 19 — `reopen_outcome` graduado a atom

Data: 2026-09-10 · Fire 777

## O que subiu

`reopen_outcome` (wal/reopen_kernel.rs, par `reopen_outcome`,
F170/F171/G8, handler `open_with_env`) saiu de `close` para `atom`
com um ∀ iff registrado em `formal/aeneas/lean/Reopen.lean`:

```lean
theorem reopen_outcome_serve_all_iff_damage_none :
    ∀ (damage : ReopenDamage) (point_in_time : Bool) (escalated : Bool),
      (reopen_outcome damage point_in_time escalated
          = ok ReopenOutcome.ServeAll)
        ↔ (damage = ReopenDamage.None)
```

0 sorry, `lake build Reopen` verde na primeira build. Planta (dente)
já verde: `reopen_outcome_on_live_crc_is_not_ok`. Wrapper já tinha os
teeth `pit_reports_unless_escalated` e `as_is_swallows_damage`.

## Por que essa statement

Os 4 construtores de dano (TruncatedHead, Crc, ZeroHeader, Resync) têm
corpos idênticos; o conteúdo semântico do par é o núcleo G8: dano
nunca é servido inteiro em silêncio. `ServeAll ⟺ damage = None` captura
isso por inteiro — nos 4 × 2 × 2 casos de dano o resultado é RefuseOpen
ou ServePrefixReport (ambos ≠ ServeAll por construtor).

## Contas (mesma commit)

- `cap_data_fate` 112 → 111; `floor_atom` 19 → 20; `floor_extract` 260 → 259
- `close_proofs.tsv`: linha `atom catalog:reopen_outcome …`
- gates `check_depth_floor.py` + `check_product_floor.py` GREEN
  (`data_fate=111<=111`)

## Lore

Sem lore nova — forma 776 com o match já no `do` de nível único
desdobra por `cases damage` + `cases point_in_time <;> cases escalated
<;> simp at h`; backward de dano é `absurd habsurd (by simp)`
(construtores distintos). Verde na primeira build.

## Estado da fila (após este passo)

Faltam 11 descidas para cap ≤ 100; fila viva = 9 → melhor caso cap 102
(déficit 2 confirmado; plano de cobertura no diário Fire 775).
