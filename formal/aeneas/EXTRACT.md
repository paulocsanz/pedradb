# Aeneas extract + Lean theorems

**Date:** 2026-08-15

## Vote (`vote_kernel.rs`)

- Charon `0.1.232` + Aeneas `daa85d7` → `out/lean/VoteKernel.lean`.
- Production tweaks so the extract is not an axiom: `can_vote` and `grant_after_persist` use `match`.
- Lean 4.31.0 accepted (no `sorry` in `Vote.lean`):
  - `vote_decision_matches_spec`
  - `grant_after_persist_implies_ok` (wire grant ⇒ persist Ok)
  - `as_is_grants_where_fixed_denies` (mutant teeth)

## Isolated (F83, was a Verus cartoon)

- `[lib] path` = production `isolated_kernel.rs`.
- `starts_with` extracted as an axiom; kernel rewritten to a byte loop (same predicate). Re-extract is **axiom-free**.
- Lean 4.31.0 accepted (no `sorry` in `Isolated.lean`):
  - `isolated_id_matches_loop_spec` / `_spec` — ∀ prefix + (`=` ∨ next `'/'`)
  - `isolated_id_matches_as_is_loop_spec` / `_spec` — ∀ prefix only
  - `as_is_leaks_sibling` — `/vm/vm-a` vs `/vm/vm-ab` (F83 teeth)
  - `isolated_id_matches_too_short`, child-byte atoms
- Method that closed: `loop.spec_decr_nat` + `step` on `index_usize_spec`;
  lengths only as `x.val.length : Nat`; no `simp [loop]`.

## What we may say

> Lean accepted those named theorems of the Aeneas extracts of the production Rust files. Persist/disk remain axioms. F83 sibling is now Lean-∀ (`as_is_leaks_sibling`), not only Stateright.

Never: “Lean proved Raft / fold.”

## Re-check

```
./scripts/aeneas_vote.sh --required
./scripts/aeneas_isolated.sh --required
./scripts/lean_vote.sh --required
```
