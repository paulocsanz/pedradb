# RFC-0191 P2.3 passo 18 — `compact_decision` graduado a atom

Data: 2026-09-10 · Fire 776

## O que subiu

`compact_pick` (compact_kernel.rs, entry do par `compact_decision`,
F177/F20, handler `compact_with_ssts_only`) saiu de `close` para `atom`
com um ∀ iff registrado em `formal/aeneas/lean/Compact.lean`:

```lean
theorem compact_pick_gc_rewrite_max_iff_none_and_gc_and_files_at_max :
    ∀ (lowest_level_with_files : Option U32) (files_at_max_level : Bool)
      (gc_requested : Bool) (max_level : U32),
      (compact_pick lowest_level_with_files files_at_max_level gc_requested max_level
          = ok CompactPlan.GcRewriteMax)
        ↔ (lowest_level_with_files = none ∧ gc_requested = true
            ∧ files_at_max_level = true)
```

0 sorry, `lake build Compact` verde. Planta (dente) já verde:
`compact_pick_on_live_merge_is_not_ok` (+ `theorem_compact_pick_on_finite_domain`).

## Contas (mesma commit)

- `cap_data_fate` 113 → 112 (TSV e residuals)
- `floor_atom` 18 → 19; `floor_extract` 261 → 260 (recount da promoção)
- `close_proofs.tsv`: linha `atom catalog:compact_decision …`
- gates `check_depth_floor.py` + `check_product_floor.py` GREEN
  (`data_fate=112<=112`)

## Lore nova (Aeneas/Lean)

1. **`Std.U32` não existe no contexto do wrapper** — `open Aeneas.Std`
   deixa o nome bare (`U32`/`U64`); escrever `Std.U32` falha elaboração
   e CASCATA: cada erro seguinte (`unsolved goals`, `generalize failed:
   result is not type correct`) é artefato do binder quebrado, não
   tactic bug. Sintoma característico: variáveis exibidas como `sorry`.
2. **`cases` sobre o scrutinee NÃO iota-reduz o `match` em `h`** — o
   match fica exibido inteiro e o `l + 1#u32` do branch some sob o
   binder do próprio match (não é o `l` livre). `rw [hadd] at h` acha
   zero ocorrências. Cura: `simp at h` PRIMEIRO (simp reduz o match por
   iota), só depois `cases hadd : l + 1#u32` + `rw [hadd] at h; simp at h`.
3. **`rintro ⟨rfl, …⟩` não aceita `none = none`** (nenhum lado é
   variável) — usar `-` para limpar: `rintro ⟨-, rfl, rfl⟩`.
4. Aritmética de máquina Aeneas é `Result` com 3 construtores
   (`ok`/`fail`/`div`), e `bind_ok`/`bind_fail`/`bind_div` são
   `@[simp]` — o three-way `cases hadd :` fecha cada ramo sozinho.

## Estado da fila (após este passo)

Faltam 12 descidas para cap ≤ 100; fila viva = 10 → melhor caso cap 102.
Ver diário Fire 775/776 para o plano de cobertura do déficit (26 pares
sem campo `aeneas`, 10 entry-missing — 7 no L28Kernel).
